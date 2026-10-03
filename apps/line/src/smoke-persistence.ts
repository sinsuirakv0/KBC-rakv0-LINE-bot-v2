import assert from "node:assert/strict";
import { createCipheriv, createHash, randomBytes } from "node:crypto";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { GitHubPersistence } from "./adapter/persistence.js";
import { AuthStorage } from "./adapter/storage.js";
import { createCore, PROTOCOL_VERSION } from "./protocol/native.js";

// 外部通信なしでコンテナ交換・sequence予約・保存失敗を確認する。
const files = new Map<string, Buffer>();
let writes = 0, rejectWrites = false;
const request = (async (url: string | URL | Request, init?: RequestInit) => {
  const path = new URL(String(url)).pathname.split("/contents/")[1];
  if (!path) return Response.json({ private: true });
  if (init?.method === "PUT") {
    if (rejectWrites) return new Response("", { status: 503 });
    files.set(path, Buffer.from(JSON.parse(String(init.body)).content, "base64"));
    writes++;
    return Response.json({ content: { sha: String(writes) } });
  }
  const data = files.get(path);
  if (!data) return new Response("", { status: 404 });
  return new Headers(init?.headers).get("Accept")?.includes("raw") ? new Response(new Uint8Array(data)) :
    Response.json({ sha: String(writes) });
}) as typeof fetch;
const secret = "offline-backup-fixture", iv = randomBytes(12);
const cipher = createCipheriv("aes-256-gcm", createHash("sha256").update(secret).digest(), iv);
const encrypted = Buffer.concat([cipher.update(JSON.stringify({ ".auth": "fixture-token", reqseq: '{"square":7}' })), cipher.final()]);
files.set("line-auth/storage.enc.json", Buffer.from(JSON.stringify({ version: 1, algorithm: "aes-256-gcm",
  iv: iv.toString("base64"), authTag: cipher.getAuthTag().toString("base64"), ciphertext: encrypted.toString("base64") })));
const directory = await mkdtemp(join(tmpdir(), "kbc-backup-smoke-"));
const backup = (dir: string) => new GitHubPersistence("fixture/private", "fixture", "main", secret,
  join(dir, "auth.json"), join(dir, "core.sqlite"), request);
const first = backup(directory);
assert.equal(await first.restore(), false);
const storage = new AuthStorage(join(directory, "auth.json"), () => {}, data => first.protectAuth(data));
await storage.load();
const leases = writes;
await storage.set("reqseq", '{"square":8}');
assert.equal(writes, leases);
const core = createCore({ databasePath: join(directory, "core.sqlite"), ownerId: "fixture-account" });
core.submitBatch({ protocolVersion: PROTOCOL_VERSION, streamKey: "fixture", checkpoint: "saved-checkpoint", baselineBeforeMs: null, events: [{
  type: "messageReceived", eventId: "fixture-message", chatId: "m" + "1".repeat(32),
  messageId: "fixture-message", senderId: "p" + "1".repeat(32), text: "!ping", createdAtMs: Date.now() - 100,
}] });
await first.backup(core);
const backedUp = writes;
await first.backup(core);
assert.equal(writes, backedUp);
core.shutdown();
const replacement = join(directory, "replacement");
const second = backup(replacement);
assert.equal(await second.restore(), true);
const restoredAuth = JSON.parse(await readFile(join(replacement, "auth.json"), "utf8"));
assert.equal(JSON.parse(restoredAuth.reqseq).square, 10007);
const restored = createCore({ databasePath: join(replacement, "core.sqlite"), ownerId: "fixture-account", restoredFromBackup: true });
assert.equal(restored.checkpoint("fixture"), "saved-checkpoint");
assert.equal(restored.stats().unknownActions, 1);
rejectWrites = true;
let aborted = false;
const failed = new AuthStorage(join(replacement, "auth.json"), () => { aborted = true; }, data => second.protectAuth(data));
await failed.load();
await assert.rejects(failed.set("reqseq", '{"square":30000}'), /AuthStorageError/);
assert.equal(aborted, true);
assert.equal(JSON.parse(JSON.parse(await readFile(join(replacement, "auth.json"), "utf8")).reqseq).square, 10007);
restored.shutdown();
console.log(JSON.stringify({ ok: true, cases: 3, externalRequests: 0,
  restoredCheckpoint: true, uncertainWritesHeld: true, sequenceReusePrevented: true, failedReservationStopsTransport: true }));
