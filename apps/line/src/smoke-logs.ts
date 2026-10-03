import assert from "node:assert/strict";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gunzipSync } from "node:zlib";
import { createCore, PROTOCOL_VERSION } from "./protocol/native.js";
import { GitHubPersistence } from "./adapter/persistence.js";
import { LogSync } from "./adapter/logs.js";
import { normalizeEvents } from "./adapter/events.js";
import { Receiver } from "./adapter/receiver.js";
import { ApiScheduler } from "./adapter/api.js";
import { BaseClient } from "@evex/linejs/base";
import type { CoreEvent } from "./protocol/generated/CoreEvent.js";

// 実通信なしで受付とログ確定の境界、追記失敗後の再開を確認する。
const root = await mkdtemp(join(tmpdir(), "kbc-log-smoke-"));
const config = { databasePath: join(root, "core.sqlite"), ownerId: "fixture", logsEnabled: true };
let core = createCore(config);
const files = new Map<string, { data: Buffer; sha: string }>();
let sequence = 0, failManifest = false, conflicts = 0;
const request = (async (url: string | URL | Request, init?: RequestInit) => {
  const path = new URL(String(url)).pathname.split("/contents/")[1];
  if (!path) throw new Error("UnexpectedFixtureRequest");
  if (init?.method === "PUT") {
    if (failManifest && path.endsWith("manifest.json")) return new Response("", { status: 503 });
    if (conflicts > 0) { conflicts--; return new Response("", { status: 409 }); }
    const payload = JSON.parse(String(init.body));
    if (payload.sha !== files.get(path)?.sha) return new Response("", { status: 409 });
    files.set(path, { data: Buffer.from(payload.content, "base64"), sha: String(++sequence) });
    return Response.json({});
  }
  const file = files.get(path);
  return file ? Response.json({ sha: file.sha, size: file.data.length, encoding: "base64", content: file.data.toString("base64") }) : new Response("", { status: 404 });
}) as typeof fetch;
const github = new GitHubPersistence("fixture/private", "fixture", "main", "fixture", "unused", "unused", request);
let sync = new LogSync(core, github);
const square = "s" + "1".repeat(32), chat = "m" + "1".repeat(32), member = "p" + "1".repeat(32);
const at = Date.now();
async function submit(events: CoreEvent[]) {
  return core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "fixture", checkpoint: String(sequence), baselineBeforeMs: null, events });
}
const first: CoreEvent = { type: "messageReceived", eventId: "first", messageId: "first", chatId: chat, squareId: square,
  senderId: member, senderName: "名前🙂", text: "本文\n次の行", createdAtMs: at, metadataJson: '{"contentMetadata":{"STKID":"7"}}' };
await submit([first]); await submit([first]);
assert.equal(core.pendingLogs().length, 2);
failManifest = true;
await assert.rejects(sync.flush(), /GitHubWrite503/);
assert.equal(core.pendingLogs().length, 2);
core.shutdown(); core = createCore(config); sync = new LogSync(core, github);
failManifest = false; conflicts = 1;
await sync.flush(); assert.equal(core.pendingLogs().length, 0);
await submit([{ ...first, eventId: "second", messageId: "second", createdAtMs: at + 1 }]);
await sync.flush();
const messages = [...files.entries()].filter(([path]) => path.includes("/messages/") && path.endsWith(".gz"));
assert.equal(messages.length, 1);
const lines = gunzipSync(messages[0][1].data).toString("utf8").trim().split("\n");
assert.equal(lines.length, 3); assert.equal(JSON.parse(lines[1])[3], first.text);
assert.equal(JSON.parse(lines[1])[6][0].contentMetadata.STKID, "7");
const changed: CoreEvent = { type: "memberChanged", eventId: "rename", squareId: square, chatId: chat, memberId: member,
  scope: "square", state: "NAME", displayName: "別の名前", createdAtMs: at + 2 };
await submit([changed]); await sync.flush();
const names = [...files.entries()].find(([path]) => path.includes("/names/") && path.endsWith(".gz"))!;
const rename = JSON.parse(gunzipSync(names[1].data).toString("utf8").trim().split("\n").at(-1)!);
assert.equal(rename[2], "名前🙂"); assert.equal(rename[3], "別の名前");
assert(![...files.keys()].some(path => path.includes("member-events")));
const kicked = await normalizeEvents({ type: "NOTIFIED_KICKOUT_FROM_SQUARE", createdTime: at + 3, payload: {
  notifiedKickoutFromSquare: { squareChatMid: chat, kickees: [member, "p" + "2".repeat(32)].map(squareMemberMid => ({squareMemberMid, squareMid: square, displayName: "参加者"})) },
} } as Parameters<typeof normalizeEvents>[0]);
assert.equal(kicked.length, 2); assert(kicked.every(event => event.type === "memberChanged" && event.state === "KICK_OUT"));
await submit(kicked); await sync.flush();
assert.equal(core.stats().pendingLogs, 0);
const previous = JSON.stringify({ originMs: at - 1000, syncToken: "before" });
await core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "page", checkpoint: previous, baselineBeforeMs: null, events: [] });
let calls = 0;
const failingCore = { ...core, submitBatchAsync: async (batch: Parameters<typeof core.submitBatchAsync>[0]) => {
  if (++calls === 2) throw new Error("FixtureFailure");
  return core.submitBatchAsync(batch);
} };
const controller = new AbortController();
const receiver = new Receiver(new BaseClient({ device: "DESKTOPWIN" }), failingCore, new ApiScheduler(controller.signal, 2, 1), controller.signal);
const accept = (receiver as unknown as { accept: (stream: string, checkpoint: {originMs:number;syncToken:string}, events: Parameters<typeof normalizeEvents>[0][]) => Promise<void> }).accept.bind(receiver);
const page = [at + 10, at + 11].map(createdTime => ({ type: "NOTIFIED_KICKOUT_FROM_SQUARE", createdTime, payload: {
  notifiedKickoutFromSquare: { squareChatMid: chat, kickees: Array.from({length:64}, (_, i) => ({ squareMid: square,
    squareMemberMid: "p" + (i + 10).toString(16).padStart(32, "0"), displayName: "参加者" })) },
} } as Parameters<typeof normalizeEvents>[0]));
await assert.rejects(accept("page", { originMs: at - 1000, syncToken: "after" }, page));
assert.equal(core.checkpoint("page"), previous);
await accept("page", { originMs: at - 1000, syncToken: "after" }, page);
assert.equal(JSON.parse(core.checkpoint("page")!).syncToken, "after");
for (let start = 0; start < 180; start += 8) {
  await submit(Array.from({ length: Math.min(8, 180 - start) }, (_, i) => ({ ...first,
    eventId: `large-${start + i}`, messageId: `large-${start + i}`, text: "x".repeat(30000), createdAtMs: at + 20 + start + i })));
}
await sync.flush();
const combined = [...files.entries()].filter(([path]) => path.includes("/messages/") && path.endsWith(".gz"));
assert.equal(combined.length, 2);
assert.equal(combined.reduce((n, [, file]) => n + gunzipSync(file.data).toString("utf8").trim().split("\n").length - 1, 0), 182);
core.shutdown();
console.log(JSON.stringify({ ok: true, scenarios: ["same-file-append-and-size-rollover", "manifest-failure-restart-dedup-conflict", "name-change-and-multi-kick", "expanded-page-partial-checkpoint-recovery"], externalRequests: 0 }));
