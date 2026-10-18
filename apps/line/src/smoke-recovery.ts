import assert from "node:assert/strict";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { BaseClient } from "@evex/linejs/base";
import { LINEStruct } from "@evex/linejs/thrift";
import { TCompactProtocol } from "thrift";
import { ApiScheduler, authenticationFailed } from "./adapter/api.js";
import { LineRecovery } from "./adapter/recovery.js";
import { Receiver } from "./adapter/receiver.js";
import { deliverAction } from "./adapter/delivery.js";
import { createCore, PROTOCOL_VERSION } from "./protocol/native.js";

// 実認証を使わず、停止・3回・監視周期・購読確認後の再開を1本で再生する。
const controller = new AbortController(), gate = new ApiScheduler(controller.signal, 2, 1);
// 通常RPCの完了Contextから次を起こしても、復旧要求のScopeだけを引き継ぐ。
const release: Array<() => void> = [];
const busy = [0, 1].map(() => gate.run("busy", () => new Promise<void>(resolve => { release.push(resolve); })));
await until(() => release.length === 2);
const cancelled = assert.rejects(gate.run("queued", async () => { throw new Error("ShouldNotRun"); }), /LineApiUnavailable/);
gate.pause();
const probe = gate.withProbe(() => gate.run("probe", async () => gate.beforeFetch()));
release.forEach(finish => finish());
await Promise.all([...busy, cancelled, probe]);
gate.resume();
const client = new BaseClient({ device: "DESKTOPWIN" });
const authError = Object.assign(new Error("RequestError"), { data: { errorCode: "AUTHENTICATION_FAILURE" } });
assert(authenticationFailed(authError));
assert(!authenticationFailed({ data: { code: "NOT_AUTHORIZED" } }));
await client.storage.set("refreshToken", "fixture-refresh");
client.on("update:authtoken", token => { void client.storage.set(".auth", token); });
let refreshes = 0, flushing = 0, rejectRefresh = false;
client.auth.refresh = async () => gate.run("refresh", async () => {
  if (rejectRefresh) throw authError;
  const count = ++refreshes;
  await delay(2);
  return { accessToken: `fixture-access-${count}`, refreshToken: `fixture-refresh-${count}`,
    tokenIssueTimeEpochSec: 1000, durationUntilRefreshInSec: 100 } as Awaited<ReturnType<typeof client.auth.refresh>>;
});
const recovery = new LineRecovery(client, gate, controller.signal, async () => {
  flushing++;
  assert.equal(await client.storage.get("refreshToken"), `fixture-refresh-${refreshes}`);
  assert.equal(await client.storage.get(".auth"), client.authToken);
}, 5, 150);
await Promise.all([client.auth.tryRefreshToken(), client.auth.tryRefreshToken()]);
assert.equal(refreshes, 1); assert.equal(flushing, 1);
const directory = await mkdtemp(join(tmpdir(), "kbc-recovery-smoke-"));
const core = createCore({ databasePath: join(directory, "core.sqlite"), ownerId: "fixture" });
const originMs = Date.now() - 1000;
core.submitBatch({ protocolVersion: PROTOCOL_VERSION, streamKey: "account",
  checkpoint: JSON.stringify({ syncToken: "durable", originMs }), baselineBeforeMs: null, events: [{
    type: "messageReceived", eventId: "chat:queued", messageId: "queued", chatId: "chat", text: "!ping", createdAtMs: Date.now(),
  }] });
let normalCalls = 0, sends = 0;
client.square.fetchMyEvents = async () => gate.run("fetchMyEvents", async () => {
  normalCalls++;
  return { events: [], syncToken: "directory", continuationToken: "", subscription: { subscriptionId: 1, ttlMillis: 300000 } };
});
client.square.sendMessage = async () => gate.run("sendMessage", async () => {
  gate.beforeFetch(); sends++;
  return { createdSquareMessage: { message: { id: "sent" } } } as Awaited<ReturnType<typeof client.square.sendMessage>>;
});
const push = client.push;
let connections = 0, reading: Promise<void>, finishRead = () => {}, allowReconnect = false;
let lastSignOn = () => {};
const connectedAt: number[] = [];
push.initializeConn = async () => {
  connections++; connectedAt.push(Date.now());
  assert.equal(client.authToken, await client.storage.get(".auth"));
  if (connections > 1) assert.equal(JSON.parse(core.checkpoint("account")!).syncToken, "durable");
  reading = new Promise(resolve => { finishRead = resolve; });
  push.signOnRequests[1] = [3];
  return { resStream: new ReadableStream(), cacheData: new Uint8Array(), notFinPayloads: {},
    onDataReceived: () => {}, writeByte: async () => {}, close: async () => { finishRead(); },
  } as unknown as Awaited<ReturnType<typeof push.initializeConn>>;
};
push.InitAndRead = async () => {
  if (connections > 1 && !allowReconnect) throw authError;
  lastSignOn = () => push.onSignOnResponse(1, true, client.thrift.writeThrift([[12, 0, LINEStruct.FetchMyEventsResponse({
    events: [], syncToken: "durable", subscription: { subscriptionId: 1, ttlMillis: 300000 },
  })]], "fetchMyEvents", TCompactProtocol));
  if (connections === 1) lastSignOn();
  return reading;
};
const receiver = new Receiver(client, core, gate, controller.signal, undefined, recovery);
const running = receiver.run();
async function until(condition: () => boolean) {
  const deadline = Date.now() + 3000;
  while (!condition()) { assert(Date.now() < deadline, "RecoveryTimedOut"); await delay(2); }
}
try {
  await until(() => receiver.status === "receiving");
  await assert.rejects(gate.run("fetchSquareChatEvents", async () => { throw authError; }));
  const callsAtPause = normalCalls;
  const action = await core.nextAction(); assert(action);
  assert.equal((await deliverAction(client, core, gate, action)).status, "queued");
  assert.equal(sends, 0); assert.equal(core.stats().unknownActions, 0);
  let resumed = false;
  const ready = gate.waitReady().then(() => { resumed = true; });
  await until(() => recovery.state === "api-suspended");
  assert.equal(recovery.snapshot().attempts, 3);
  assert.equal(connections, 4); assert.equal(refreshes, 4);
  const stoppedAt = Date.now();
  await delay(30);
  assert.equal(connections, 4); assert.equal(normalCalls, callsAtPause);
  assert.equal(resumed, false);
  allowReconnect = true; rejectRefresh = true;
  await until(() => connections === 5);
  assert(connectedAt[4] - stoppedAt >= 100);
  assert.equal(recovery.snapshot().checks, 1);
  assert.equal(receiver.status, "api-suspended"); assert.equal(resumed, false);
  await assert.rejects(gate.run("ordinary", async () => { normalCalls++; }), /LineApiUnavailable/);
  lastSignOn();
  await ready; await until(() => receiver.status === "receiving");
  assert.equal(recovery.state, "ready");
  assert.equal(refreshes, 4);
  await delay(1000);
  const retry = await core.nextAction(); assert(retry && retry.actionId === action.actionId);
  assert.equal((await deliverAction(client, core, gate, retry)).status, "sent");
  assert.equal(sends, 1);
  await assert.rejects(gate.run("oneOc", async () => { throw { data: { errorCode: "FORBIDDEN" } }; }));
  assert.equal(recovery.state, "ready");
  // 停止中の終了も待機を解除し、監視Timerを残さない。
  recovery.notice(authError);
  const blocked = gate.waitReady();
  controller.abort();
  await assert.rejects(blocked);
} finally { controller.abort(); await running; core.shutdown(); }
console.log(JSON.stringify({ ok: true, scenarios: ["refresh-single-flight", "three-attempts-minute-monitor", "square-sign-on-readiness", "unsent-outbox-resume", "stop-abort"] }));
