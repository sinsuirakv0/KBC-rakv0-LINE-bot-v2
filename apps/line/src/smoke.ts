import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { setTimeout as delay } from "node:timers/promises";
import { createCore, PROTOCOL_VERSION, type CoreConfig, type ReceivedBatch } from "./protocol/native.js";
import { ApiScheduler, installApiScheduler } from "./adapter/api.js";
import { AuthStorage } from "./adapter/storage.js";
import { deliverAction } from "./adapter/delivery.js";
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
  core.markSending(first.actionId);
  core.completeAction({ actionId: first.actionId, status: "sent", code: "OK" });
  const second = await core.nextAction();
  assert(second);
  assert.notEqual(first.actionId, second.actionId);
  core.markSending(second.actionId);
  assert.throws(() => core.retryAction(second.actionId, 0), /ActionNotClaimed/);
  const unstartedBatch = batch(["unstarted"]);
  unstartedBatch.events[0] = { ...unstartedBatch.events[0], chatId: "other" };
  core.submitBatch(unstartedBatch);
  const unstarted = await core.nextAction();
  assert(unstarted);
  // 再起動時、取り出し済みは再待機、通信開始済みだけ結果不明になる。
  core.submitBatch(batch(["timer"], "o.test-notify 1", "cursor-2"));
  core.shutdown();
  core = createCore(config);
  assert.equal(core.checkpoint("account"), "cursor-2");
  assert.equal(core.stats().unknownActions, 1);
  assert.equal(core.stats().claimedActions, 0);
  const reclaimed = await core.nextAction();
  assert(reclaimed && reclaimed.actionId === unstarted.actionId);
  core.markSending(reclaimed.actionId);
  core.completeAction({ actionId: reclaimed.actionId, status: "sent", code: "OK" });
  const acknowledgment = await core.nextAction();
  assert(acknowledgment && acknowledgment.text.includes("1秒後"));
  core.markSending(acknowledgment.actionId);
  core.completeAction({ actionId: acknowledgment.actionId, status: "sent", code: "OK" });
  // 以後一切入力しない。Coreの期限だけで通知が進む。
  const notification = await Promise.race([core.nextAction(), delay(3000).then(() => { throw new Error("NotificationStalled"); })]);
  assert(notification && notification.text.includes("次の入力"));
  core.markSending(notification.actionId);
  core.completeAction({ actionId: notification.actionId, status: "sent", code: "OK" });
  core.resolveAction({ actionId: second.actionId, status: "failed", code: "OperatorConfirmed" });
  assert.equal(core.stats().unknownActions, 0);
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

// 完了履歴が満杯でも新規配送を受付し、本文を持たない重複IDは8,192件を越えて保持する。
const capacityPath = join(directory, "capacity.sqlite");
const capacityCore = createCore({ ...config, databasePath: capacityPath });
const fixture = new DatabaseSync(capacityPath);
fixture.exec("BEGIN");
const insertEvent = fixture.prepare("INSERT INTO events VALUES (?,'',?)");
for (let index = 0; index < 9000; index++) insertEvent.run(`fixture-${index}`, now);
const insertAction = fixture.prepare("INSERT INTO actions(id,event_id,chat,payload,due,created,status,completed) VALUES (?,?,?,'',?,?,'sent',?)");
for (let index = 0; index < 2048; index++) insertAction.run(`completed-${index}`, `fixture-${index}`, "fixture", now, now, now);
fixture.exec("COMMIT");
fixture.close();
try {
  assert.equal(capacityCore.submitBatch(batch(["after-capacity"])).actionsCreated, 1);
  const action = await capacityCore.nextAction();
  assert(action);
  capacityCore.markSending(action.actionId);
  capacityCore.completeAction({ actionId: action.actionId, status: "sent", code: "OK" });
  assert.equal(capacityCore.stats().completedActions, 2048);
  assert.equal(capacityCore.submitBatch(batch(["after-capacity"])).duplicates, 1);
  const unresolved = new DatabaseSync(capacityPath);
  unresolved.exec("BEGIN");
  const insertUnknown = unresolved.prepare("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES (?,?,?,'{}',?,?,'unknown')");
  for (let index = 0; index < 2048; index++) insertUnknown.run(`unknown-${index}`, `fixture-${index}`, "fixture", now, now);
  unresolved.exec("COMMIT");
  unresolved.close();
  assert.throws(() => capacityCore.submitBatch(batch(["must-rollback"], "o.ping", "must-not-advance")), /StoreCapacity/);
  assert.equal(capacityCore.checkpoint("account"), "cursor-1");
  assert.equal(capacityCore.stats().retainedEvents, 9001);
} finally { capacityCore.shutdown(); }

// 実SDKのreqseq書込を失敗させる。通信0件で待機へ戻り、保存障害は全体停止へ伝わる。
const failedCore = createCore({ ...config, databasePath: join(directory, "delivery.sqlite") });
const failedController = new AbortController();
const blockedPath = join(directory, "not-a-directory");
await writeFile(blockedPath, "fixture");
const failedStorage = new AuthStorage(join(blockedPath, "auth.json"), error => failedController.abort(error));
let networkCalls = 0;
const failedClient = new BaseClient({ device: "DESKTOPWIN", storage: failedStorage,
  fetch: async () => { networkCalls++; throw new Error("NetworkUnknown"); } });
const failedGate = new ApiScheduler(failedController.signal, 2, 1);
installApiScheduler(failedClient, failedGate, failedController.signal);
failedCore.submitBatch(batch(["before-network"]));
try {
  const action = await failedCore.nextAction();
  assert(action);
  assert.equal((await deliverAction(failedClient, failedCore, failedGate, action)).status, "queued");
  assert(failedController.signal.aborted);
  assert.equal(networkCalls, 0);
  assert.equal(failedCore.stats().unknownActions, 0);
  await assert.rejects(failedStorage.set("later", "fixture"), /AuthStorageError/);
  await assert.rejects(failedStorage.flush(), /AuthStorageError/);
  // 別の健全な保存で再開し、実transport到達後の例外は結果不明として一度だけ記録する。
  const networkController = new AbortController();
  const networkClient = new BaseClient({ device: "DESKTOPWIN", fetch: async () => {
    networkCalls++; assert.equal(failedCore.stats().sendingActions, 1); throw new Error("NetworkUnknown");
  } });
  const networkGate = new ApiScheduler(networkController.signal, 2, 1);
  installApiScheduler(networkClient, networkGate, networkController.signal);
  const retry = await failedCore.nextAction();
  assert(retry && retry.actionId === action.actionId);
  assert.equal((await deliverAction(networkClient, failedCore, networkGate, retry)).status, "unknown");
  assert.equal(networkCalls, 1);
  assert.equal(failedCore.stats().queuedActions, 0);
  networkController.abort();
} finally { failedCore.shutdown(); }

// 実SDKのThrift初期応答を使い、継続中のPUSH通知・必要なチャット補完を再生する。
type AccountPage = Awaited<ReturnType<BaseClient["square"]["fetchMyEvents"]>>;
const replayConfig = { databasePath: join(directory, "receiver.sqlite"), ownerId: "test-account" };
let replayCore = createCore(replayConfig);
const replayController = new AbortController();
const client = new BaseClient({ device: "DESKTOPWIN" });
const push = client.push;
let finishRead!: () => void;
let reading: Promise<void>;
const connection = {
  resStream: new ReadableStream(), cacheData: new Uint8Array(), notFinPayloads: {},
  onDataReceived: () => {}, writeByte: async () => {}, close: async () => { finishRead(); },
} as unknown as Awaited<ReturnType<typeof push.initializeConn>>;
push.initializeConn = async () => {
  reading = new Promise<void>(resolve => { finishRead = resolve; });
  push.signOnRequests[1] = [3]; return connection;
};
push.InitAndRead = async () => {
  const data = client.thrift.writeThrift([[12, 0, LINEStruct.FetchMyEventsResponse({
    subscription: { subscriptionId: 1, ttlMillis: 300000 }, events: [],
    syncToken: JSON.parse(replayCore.checkpoint("account") ?? "{}").syncToken ?? "baseline",
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
    return page([first, message("b"), { type: "NOTIFICATION_MESSAGE", payload: { notificationMessage: {
      squareChatMid: "broken", requiredToFetchChatEvents: true,
    } } }], "c1", "next-page");
  }
  if (accountCalls === 2) { assert.equal(options.continuationToken, "next-page"); return page([message("c")], "c2"); }
  return page([], "c3");
};
let chatCalls = 0;
let brokenCalls = 0;
let brokenRecovered = false;
(client.request as unknown as { request: (...args: unknown[]) => Promise<AccountPage> }).request = async (...args) => {
  if (JSON.stringify(args[0]).includes('"broken"')) {
    brokenCalls++;
    if (!brokenRecovered) throw new Error("ChatUnavailable");
    const recovered = message("e");
    recovered.payload.notificationMessage.squareMessage.message.to = "broken";
    return page([recovered], "broken-c1");
  }
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
  assert.equal(brokenCalls, 1);
  assert.deepEqual(JSON.parse(replayCore.checkpoint("account")!).pendingChats, ["broken"]);
} finally { replayController.abort(); await receiving; replayCore.shutdown(); }

// 未完了トークと待機期限を再起動で引き継ぎ、入力なしの期限再試行で復旧する。
replayCore = createCore(replayConfig);
const resumedController = new AbortController();
const resumed = new Receiver(client, replayCore, new ApiScheduler(resumedController.signal, 2, 1), resumedController.signal);
const resuming = resumed.run();
try {
  const deadline = Date.now() + 4000;
  while (resumed.status !== "receiving") { assert(Date.now() < deadline); await delay(5); }
  assert.equal(brokenCalls, 1);
  const beforeRetry = accountCalls;
  brokenRecovered = true;
  while (replayCore.stats().queuedActions < 5) { assert(Date.now() < deadline); await delay(5); }
  assert.equal(brokenCalls, 2);
  assert.equal(accountCalls, beforeRetry);
  assert.deepEqual(JSON.parse(replayCore.checkpoint("account")!).pendingChats, []);
} finally { resumedController.abort(); await resuming; replayCore.shutdown(); }
console.log(JSON.stringify({ smoke: "passed", network: false, cases: ["two-distinct-ids", "dedup", "batch-rollback", "claimed-restart", "sending-restart", "explicit-unknown-resolution", "autonomous-notification", "owner-check", "bounded-api-refresh", "completed-capacity", "unresolved-capacity-rollback", "pre-send-storage-failure", "post-send-unknown", "push-sign-on", "continuation-dirty-hint", "chat-failure-isolation", "durable-chat-retry"] }));
