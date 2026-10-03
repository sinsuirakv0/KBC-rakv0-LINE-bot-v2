import { createHash } from "node:crypto";
import { gzipSync, gunzipSync } from "node:zlib";
import { setTimeout as delay } from "node:timers/promises";
import type { NativeCore } from "../protocol/native.js";
import type { PendingLog } from "../protocol/generated/PendingLog.js";
import type { GitHubPersistence } from "./persistence.js";
import { errorCode } from "./api.js";

type FileInfo = { name: string; count: number; rawBytes: number; bytes: number; sha256: string };
type Header = { v: number; kind: string; context: Record<string, unknown> };
type Manifest = Header & { files: FileInfo[]; active: string | null };
const TARGET_BYTES = 4 * 1024 * 1024;
const digest = (text: string | Buffer): string => createHash("sha256").update(text).digest("hex");

// 周期ではファイルを閉じず、同じ追記先を容量まで更新する。最新の遠隔SHAで競合を検出する。
export class LogSync {
  readonly metrics = { cycles: 0, failures: 0, rows: 0, lastSyncAt: 0 };
  constructor(private core: NativeCore, private github: GitHubPersistence) {}

  async flush(): Promise<void> {
    const groups = new Map<string, PendingLog[]>();
    for (const row of this.core.pendingLogs()) {
      if (!groups.has(row.stream) && groups.size >= 32) continue;
      const group = groups.get(row.stream) ?? [];
      group.push(row); groups.set(row.stream, group);
    }
    for (const [stream, rows] of groups) {
      let saved = false;
      for (let attempt = 0; attempt < 3 && !saved; attempt++) saved = await this.append(stream, rows);
      if (!saved) throw new Error("LogWriteConflict");
      for (let index = 0; index < rows.length; index += 128) {
        this.core.acknowledgeLogs(rows.slice(index, index + 128).map(row => row.sequence));
      }
      this.metrics.rows += rows.length;
    }
    this.metrics.cycles++;
    this.metrics.lastSyncAt = Date.now();
  }

  private async append(stream: string, incoming: PendingLog[]): Promise<boolean> {
    if (!/^(s[A-Za-z0-9]+(?:\/m[A-Za-z0-9]+)?|unmapped\/[A-Za-z0-9]+)\/(messages|member-events|names)$/.test(stream)) throw new Error("InvalidLogStream");
    const path = `logs/v2/${stream}`, manifestPath = `${path}/manifest.json`;
    const remoteManifest = await this.github.readVersionedRemote(manifestPath);
    if (remoteManifest && remoteManifest.data.length > 1024 * 1024) throw new Error("LogManifestLimit");
    const kind = stream.split("/").at(-1)!;
    const manifest: Manifest = remoteManifest ? JSON.parse(remoteManifest.data.toString("utf8")) :
      { v: 1, kind, context: {}, files: [], active: null };
    if (manifest.v !== 1 || manifest.kind !== kind || !Array.isArray(manifest.files) || manifest.files.length > 4096) throw new Error("InvalidLogManifest");
    const header = JSON.stringify({ v: 1, kind, context: manifest.context }) + "\n";
    const seen = new Set<string>();
    const unpack = (data: Buffer): string[] => {
      const raw = gunzipSync(data, { maxOutputLength: 8 * 1024 * 1024 + 65536 }).toString("utf8").split("\n");
      const actual = JSON.parse(raw.shift()!);
      if (actual.v !== 1 || actual.kind !== kind || JSON.stringify(actual.context) !== JSON.stringify(manifest.context)) throw new Error("LogHeaderMismatch");
      return raw.filter(Boolean);
    };
    // 直前の退避から復元された未確認行は、最新の確定ファイルとも照合する。
    const recent = manifest.files.slice(-3);
    for (const file of recent) {
      if (!/^\d{6}\.jsonl\.gz$/.test(file.name)) throw new Error("InvalidLogFileName");
      if (file.name === manifest.active) continue;
      const data = await this.github.readVersionedRemote(`${path}/${file.name}`);
      if (!data) throw new Error("MissingSealedLog");
      if (file.sha256 && digest(data.data) !== file.sha256) throw new Error("SealedLogChecksumMismatch");
      for (const row of unpack(data.data)) seen.add(digest(row));
    }
    let number = manifest.active ? Number(manifest.active.slice(0, 6)) : 1;
    const loadActive = async () => {
      const name = String(number).padStart(6, "0") + ".jsonl.gz";
      const remote = await this.github.readVersionedRemote(`${path}/${name}`);
      const rows = remote ? unpack(remote.data) : [];
      for (const row of rows) seen.add(digest(row));
      return { name, remote, rows, bytes: Buffer.byteLength(header) + rows.reduce((n, row) => n + Buffer.byteLength(row) + 1, 0), dirty: false };
    };
    let active = await loadActive();
    const save = async (): Promise<boolean> => {
      const raw = header + active.rows.join("\n") + "\n";
      const packed = !active.dirty && active.remote ? active.remote.data : gzipSync(raw, { level: 6 });
      if (active.dirty && !await this.github.writeVersionedRemote(`${path}/${active.name}`, packed, active.remote?.sha)) return false;
      const rawBytes = !active.dirty && active.remote ? gunzipSync(packed, { maxOutputLength: 8 * 1024 * 1024 + 65536 }).length : Buffer.byteLength(raw);
      const info = { name: active.name, count: active.rows.length, rawBytes, bytes: packed.length, sha256: digest(packed) };
      const index = manifest.files.findIndex(file => file.name === active.name);
      if (index < 0) manifest.files.push(info); else manifest.files[index] = info;
      manifest.active = active.name;
      return true;
    };
    for (const row of incoming) {
      const hash = digest(row.row);
      if (seen.has(hash)) continue;
      if (active.rows.length && active.bytes + Buffer.byteLength(row.row) + 1 > TARGET_BYTES) {
        if (!await save()) return false;
        number++; active = await loadActive();
        if (seen.has(hash)) continue;
      }
      JSON.parse(row.row);
      active.rows.push(row.row); active.bytes += Buffer.byteLength(row.row) + 1; active.dirty = true; seen.add(hash);
    }
    if (!await save()) return false;
    return this.github.writeVersionedRemote(manifestPath, Buffer.from(JSON.stringify(manifest)), remoteManifest?.sha);
  }

  async run(signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      try { await delay(300000, undefined, { signal }); }
      catch { return; }
      try { await this.flush(); }
      catch (error) {
        this.metrics.failures++;
        console.error(JSON.stringify({ kind: "logSyncFailure", code: errorCode(error) }));
      }
    }
  }
}
