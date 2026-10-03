import { createCipheriv, createDecipheriv, createHash, randomBytes } from "node:crypto";
import { createReadStream } from "node:fs";
import { access, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { createGzip, gunzipSync } from "node:zlib";
import { setTimeout as delay } from "node:timers/promises";
import type { NativeCore } from "../protocol/native.js";
import { errorCode } from "./api.js";

type AuthData = Record<string, string | number | boolean | null | object>;
const MAX_REMOTE_BYTES = 16 * 1024 * 1024;
const AUTH_PATH = "line-auth/v2-reserved-storage.enc.json";
const SNAPSHOT_PATH = "v2/runtime/core.sqlite.gz.enc.json";

export async function writePrivateFile(path: string, data: string | Buffer): Promise<void> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  await writeFile(`${path}.tmp`, data, { mode: 0o600 });
  await rename(`${path}.tmp`, path);
}

// GitHubは長期退避先。LINEのsequenceは実通信前に予約し、復元時の再利用を防ぐ。
export class GitHubPersistence {
  private high: Record<string, number> = {};
  private authToken: unknown;
  private revision = "";
  private key: Buffer;
  readonly metrics = { backups: 0, failures: 0, lastBackupAt: 0, compressedBytes: 0 };

  constructor(private repo: string, private token: string, private branch: string, secret: string,
    private authPath: string, private databasePath: string,
    private request: typeof fetch = fetch) {
    if (!/^[\w.-]+\/[\w.-]+$/.test(repo) || !token || !secret || !branch) throw new Error("InvalidBackupConfiguration");
    this.key = createHash("sha256").update(secret).digest();
  }

  static fromEnvironment(authPath: string, databasePath: string): GitHubPersistence | undefined {
    const repo = process.env.PUSH_SUBSCRIPTIONS_GITHUB_REPO;
    if (!repo) return undefined;
    return new GitHubPersistence(repo, process.env.PUSH_SUBSCRIPTIONS_GITHUB_TOKEN ?? "",
      process.env.PUSH_SUBSCRIPTIONS_GITHUB_BRANCH ?? "main", process.env.LINE_STORAGE_BACKUP_KEY ?? "",
      authPath, databasePath);
  }

  private async api(path: string, init: RequestInit = {}): Promise<Response> {
    return this.request(`https://api.github.com/repos/${this.repo}${path}`, {
      ...init, signal: AbortSignal.timeout(15000), headers: {
        Accept: "application/vnd.github+json", Authorization: `Bearer ${this.token}`,
        "X-GitHub-Api-Version": "2022-11-28", "Content-Type": "application/json", ...init.headers,
      },
    });
  }

  private fileUrl(path: string): string {
    return `/contents/${path.split("/").map(encodeURIComponent).join("/")}`;
  }

  async readRemote(path: string): Promise<Buffer | undefined> {
    const response = await this.api(`${this.fileUrl(path)}?ref=${encodeURIComponent(this.branch)}`,
      { headers: { Accept: "application/vnd.github.raw+json" } });
    if (response.status === 404) return undefined;
    if (!response.ok) throw new Error(`GitHubRead${response.status}`);
    const chunks: Buffer[] = [];
    let size = 0;
    for await (const chunk of response.body ?? []) {
      size += chunk.length;
      if (size > MAX_REMOTE_BYTES) throw new Error("BackupTooLarge");
      chunks.push(Buffer.from(chunk));
    }
    return Buffer.concat(chunks);
  }

  async writeRemote(path: string, data: Buffer, message: string): Promise<void> {
    if (data.length > MAX_REMOTE_BYTES) throw new Error("BackupTooLarge");
    for (let attempt = 0; attempt < 3; attempt++) {
      const metadata = await this.api(`${this.fileUrl(path)}?ref=${encodeURIComponent(this.branch)}`);
      if (!metadata.ok && metadata.status !== 404) throw new Error(`GitHubRead${metadata.status}`);
      const sha = metadata.ok ? (await metadata.json() as { sha: string }).sha : undefined;
      const response = await this.api(this.fileUrl(path), { method: "PUT", body: JSON.stringify({
        message, branch: this.branch, sha, content: data.toString("base64"),
      }) });
      await response.body?.cancel();
      if (response.ok) return;
      if (response.status !== 409 && response.status !== 422) throw new Error(`GitHubWrite${response.status}`);
    }
    throw new Error("GitHubWriteConflict");
  }

  private encrypt(data: Buffer): Buffer {
    const iv = randomBytes(12);
    const cipher = createCipheriv("aes-256-gcm", this.key, iv);
    const ciphertext = Buffer.concat([cipher.update(data), cipher.final()]);
    return Buffer.from(JSON.stringify({ version: 1, algorithm: "aes-256-gcm", updatedAt: Date.now(),
      iv: iv.toString("base64"), authTag: cipher.getAuthTag().toString("base64"), ciphertext: ciphertext.toString("base64") }));
  }

  private decrypt(data: Buffer): Buffer {
    const envelope = JSON.parse(data.toString("utf8").replace(/^\uFEFF/, ""));
    if (envelope.version !== 1 || envelope.algorithm !== "aes-256-gcm") throw new Error("InvalidBackupEnvelope");
    const decipher = createDecipheriv("aes-256-gcm", this.key, Buffer.from(envelope.iv, "base64"));
    decipher.setAuthTag(Buffer.from(envelope.authTag, "base64"));
    return Buffer.concat([decipher.update(Buffer.from(envelope.ciphertext, "base64")), decipher.final()]);
  }

  private sequences(data: AuthData): Record<string, number> {
    const sequences = JSON.parse(typeof data.reqseq === "string" ? data.reqseq : "{}");
    if (!sequences || typeof sequences !== "object" || Array.isArray(sequences) || Object.keys(sequences).length > 32 ||
      Object.entries(sequences).some(([name, value]) => name.length > 128 || !Number.isInteger(value) ||
        (value as number) < 0 || (value as number) > 2147470000)) throw new Error("InvalidSequenceStorage");
    return sequences;
  }

  async restore(): Promise<boolean> {
    const repo = await this.api("");
    if (!repo.ok || !(await repo.json() as { private: boolean }).private) throw new Error("BackupRepositoryMustBePrivate");
    const remoteAuth = await this.readRemote(AUTH_PATH) ??
      await this.readRemote(process.env.LINE_STORAGE_GITHUB_PATH ?? "line-auth/storage.enc.json");
    let auth: AuthData = {};
    if (remoteAuth) auth = JSON.parse(this.decrypt(remoteAuth).toString("utf8"));
    this.high = this.sequences(auth);
    let local: AuthData | undefined;
    try { local = JSON.parse(await readFile(this.authPath, "utf8")); }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    if (local) auth = local;
    const current = this.sequences(auth);
    for (const [name, value] of Object.entries(this.high)) current[name] = Math.max(value, current[name] ?? 0);
    auth.reqseq = JSON.stringify(current);
    if (!auth[".auth"] && process.env.LINE_AUTH_TOKEN) auth[".auth"] = process.env.LINE_AUTH_TOKEN;
    if (typeof auth[".auth"] !== "string" || !auth[".auth"]) throw new Error("MissingStoredAuthToken");
    if (remoteAuth || local) await writePrivateFile(this.authPath, JSON.stringify(auth));
    // 起動ごとに新しい区間を先に確保する。ローカルDBが残っていてもsequenceは巻き戻さない。
    await this.protectAuth(auth, true);
    try { await access(this.databasePath); return false; }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    const snapshot = await this.readRemote(SNAPSHOT_PATH);
    if (!snapshot) return false;
    const database = gunzipSync(this.decrypt(snapshot), { maxOutputLength: 68 * 1024 * 1024 });
    if (database.subarray(0, 16).toString() !== "SQLite format 3\0") throw new Error("InvalidDatabaseBackup");
    await writePrivateFile(this.databasePath, database);
    return true;
  }

  async restoreSettings(permissionsPath: string, settingsPath: string): Promise<void> {
    for (const [remote, local] of [["settings/permissions.json", permissionsPath],
      ["moderation/oc-moderation-settings.json", settingsPath]]) {
      const data = await this.readRemote(remote);
      if (!data) throw new Error("MissingLegacyConfiguration");
      if (data.length > 2 * 1024 * 1024) throw new Error("LegacyConfigurationTooLarge");
      JSON.parse(data.toString("utf8").replace(/^\uFEFF/, ""));
      await writePrivateFile(local, data);
    }
  }

  async protectAuth(data: AuthData, force = false): Promise<void> {
    const sequences = this.sequences(data);
    if (!force && data[".auth"] === this.authToken &&
      Object.entries(sequences).every(([name, value]) => value <= (this.high[name] ?? -1))) return;
    const high = { ...this.high };
    for (const [name, value] of Object.entries(sequences)) {
      if (force || value > (high[name] ?? -1)) high[name] = value + 10000;
    }
    await this.writeRemote(AUTH_PATH, this.encrypt(Buffer.from(JSON.stringify({ ...data, reqseq: JSON.stringify(high) }))),
      "LINE認証とsequence予約を更新");
    this.high = high;
    this.authToken = data[".auth"];
  }

  async backup(core: NativeCore): Promise<void> {
    const revision = core.persistenceRevision();
    if (revision === this.revision) return;
    const path = `${this.databasePath}.snapshot`;
    await rm(path, { force: true });
    try {
      await core.snapshotDatabase(path);
      const chunks: Buffer[] = [];
      let size = 0;
      for await (const chunk of createReadStream(path).pipe(createGzip())) {
        size += chunk.length;
        if (size > 8 * 1024 * 1024) throw new Error("CompressedDatabaseTooLarge");
        chunks.push(chunk);
      }
      await this.writeRemote(SNAPSHOT_PATH, this.encrypt(Buffer.concat(chunks)), "Core状態を退避");
      this.revision = revision;
      this.metrics.backups++;
      this.metrics.lastBackupAt = Date.now();
      this.metrics.compressedBytes = size;
    } finally { await rm(path, { force: true }); }
  }

  async run(core: NativeCore, signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      try { await delay(60000, undefined, { signal }); }
      catch { return; }
      try { await this.backup(core); }
      catch (error) {
        this.metrics.failures++;
        console.error(JSON.stringify({ kind: "backupFailure", code: errorCode(error) }));
      }
    }
  }
}
