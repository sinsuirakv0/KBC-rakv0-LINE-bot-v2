import assert from "node:assert/strict";
import { mkdtemp } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { setTimeout as delay } from "node:timers/promises";
import { createCore, PROTOCOL_VERSION, type CoreConfig, type ReceivedBatch } from "./protocol/native.js";
import { ApiScheduler } from "./adapter/api.js";
import { BaseClient } from "@evex/linejs/base";
import { LINEStruct } from "@evex/linejs/thrift";
import { TCompactProtocol } from "thrift";
import { Receiver } from "./adapter/receiver.js";

const directory = await mkdtemp(join(tmpdir(), "kbc-line-smoke-"));
const config: CoreConfig = { databasePath: join(directory, "core.sqlite"), ownerId: "test-account" };
let core = createCore(config);
let now = Date.now();
function batch(ids: string[], text = "o.ping", checkpoint = "cursor-1"): ReceivedBatch {
  return { protocolVersion: PROTOCOL_VERSION, streamKey: "account", checkpoint, baselineBeforeMs: null,
    events: ids.map(id => ({ type: "messageReceived", eventId: `chat:${id}`, messageId: id, chatId: "chat", text, createdAtMs: now })) };
}
try {
  // 同本文・同時刻の異なるIDを処理し、再送したBatchは重複として扱う。
  assert.deepEqual(core.submitBatch(batch(["one", "two"])), { accepted: 2, duplicates: 0, actionsCreated: 2 });
  assert.equal(core.submitBatch(batch(["one", "two"])).duplicates, 2);
  const invalid = batch(["rollback", "invalid"], "o.ping", "bad-cursor");
  invalid.events[1] = { ...invalid.events[1], messageId: "" };
  assert.throws(() => core.submitBatch(invalid), /InvalidEvent/);
  assert.equal(core.checkpoint("account"), "cursor-1");
  assert.equal(core.stats().retainedEvents, 2);
  const first = await core.nextAction();
  assert(first);
  core.completeAction({ actionId: first.actionId, status: "sent", code: "OK" });
  const second = await core.nextAction();
  assert(second);
  assert.notEqual(first.actionId, second.actionId);
  // 再起動時、送信開始済みは結果不明、まだ送信していない予定は復元する。
  core.submitBatch(batch(["timer"], "o.test-notify 1", "cursor-2"));
  core.shutdown();
  core = createCore(config);
  assert.equal(core.checkpoint("account"), "cursor-2");
  assert.equal(core.stats().unknownActions, 1);
  const acknowledgment = await core.nextAction();
  assert(acknowledgment && acknowledgment.text.includes("1秒後"));
  core.completeAction({ actionId: acknowledgment.actionId, status: "sent", code: "OK" });
  // 以後一切入力しない。Coreの期限だけで通知が進む。
  const notification = await Promise.race([core.nextAction(), delay(3000).then(() => { throw new Error("NotificationStalled"); })]);
  assert(notification && notification.text.includes("次の入力"));
  core.completeAction({ actionId: notification.actionId, status: "sent", code: "OK" });
  const waiting = core.nextAction();
  core.shutdown();
  assert.equal(await waiting, null);
  assert.throws(() => createCore({ ...config, ownerId: "different-account" }), /OwnerMismatch/);
} finally { core.shutdown(); }

const controller = new AbortController();
const gate = new ApiScheduler(controller.signal, 2, 10);
let active = 0;
let maximum = 0;
await Promise.all(Array.from({ length: 8 }, () => gate.run("fakeRpc", async () => {
  active++; maximum = Math.max(maximum, active);
  await gate.run("fakeRefresh", () => delay(20));
  active--;
})));
assert(maximum <= 2);
assert.equal(gate.metrics.requests, 16);
controller.abort();

// 実SDKのThrift初期応答を使い、継続中のPUSH通知・必要なチャット補完を再生する。
type AccountPage = Awaited<ReturnType<BaseClient["square"]["fetchMyEvents"]>>;
const replayCore = createCore({ databasePath: join(directory, "receiver.sqlite"), ownerId: "test-account" });
const replayController = new AbortController();
const client = new BaseClient({ device: "DESKTOPWIN" });
const push = client.push;
let finishRead!: () => void;
const reading = new Promise<void>(resolve => { finishRead = resolve; });
const connection = {
  resStream: new ReadableStream(), cacheData: new Uint8Array(), notFinPayloads: {},
  onDataReceived: () => {}, writeByte: async () => {}, close: async () => { finishRead(); },
} as unknown as Awaited<ReturnType<typeof push.initializeConn>>;
push.initializeConn = async () => { push.signOnRequests[1] = [3]; return connection; };
push.InitAndRead = async () => {
  const data = client.thrift.writeThrift([[12, 0, LINEStruct.FetchMyEventsResponse({
    subscription: { subscriptionId: 1, ttlMillis: 300000 }, events: [], syncToken: "baseline",
  })]], "fetchMyEvents", TCompactProtocol);
  push.onSignOnResponse(1, true, data);
  return reading;
};
const message = (id: string) => ({ type: "NOTIFICATION_MESSAGE", payload: { notificationMessage: {
  squareChatMid: "chat", squareMessage: { message: { to: "chat", id, text: "o.ping", createdTime: Date.now() + 100 } },
} } });
const page = (events: unknown[], syncToken: string, continuationToken = "") => ({
  events, syncToken, continuationToken, subscription: { subscriptionId: 1, ttlMillis: 300000 },
}) as AccountPage;
const hint = () => push.onPushResponse({ serviceType: 3 } as Parameters<typeof push.onPushResponse>[0]);
let accountCalls = 0;
client.square.fetchMyEvents = async options => {
  accountCalls++;
  if (accountCalls === 1) {
    hint(); hint();
    await delay(10);
    const first = message("a");
    Object.assign(first.payload.notificationMessage, { requiredToFetchChatEvents: true });
    return page([first, message("b")], "c1", "next-page");
  }
  if (accountCalls === 2) { assert.equal(options.continuationToken, "next-page"); return page([message("c")], "c2"); }
  return page([], "c3");
};
let chatCalls = 0;
(client.request as unknown as { request: () => Promise<AccountPage> }).request = async () => {
  chatCalls++;
  return page([message(chatCalls === 1 ? "a" : "d")], `chat-${chatCalls}`, chatCalls === 1 ? "chat-next" : "");
};
const receiver = new Receiver(client, replayCore, new ApiScheduler(replayController.signal, 2, 1), replayController.signal);
const receiving = receiver.run();
try {
  const deadline = Date.now() + 3000;
  while (receiver.status !== "receiving") { assert(Date.now() < deadline); await delay(5); }
  hint();
  while (accountCalls < 3 || replayCore.stats().queuedActions < 4) { assert(Date.now() < deadline); await delay(5); }
  assert.equal(replayCore.stats().queuedActions, 4);
  assert.equal(receiver.metrics.duplicates, 1);
  assert.equal(chatCalls, 2);
} finally { replayController.abort(); await receiving; replayCore.shutdown(); }
console.log(JSON.stringify({ smoke: "passed", network: false, cases: ["two-distinct-ids", "dedup", "batch-rollback", "restart", "autonomous-notification", "owner-check", "bounded-api-refresh", "push-sign-on", "continuation-dirty-hint", "chat-catch-up"] }));
