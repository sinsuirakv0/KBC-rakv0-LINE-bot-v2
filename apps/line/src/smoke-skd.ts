import assert from "node:assert/strict";
import { BaseClient } from "@evex/linejs/base";
import { ApiScheduler } from "./adapter/api.js";
import { deliverAction } from "./adapter/delivery.js";
import type { ActionResult, CoreAction, NativeCore } from "./protocol/native.js";

const controller = new AbortController();
const gate = new ApiScheduler(controller.signal, 2, 1);
const client = new BaseClient({ device: "DESKTOPWIN" });
const results: ActionResult[] = [];
let marks = 0, lookups = 0, sends = 0, ordinarySends = 0, failLookup = false, failSend = false;
const core = { markSending: () => { marks++; }, completeAction: (result: ActionResult) => results.push(result),
  retryAction: () => { throw new Error("UnexpectedSideEffectRetry"); } } as unknown as NativeCore;
const action: CoreAction = { type: "sendMessage", actionId: "part", eventId: "event", chatId: "chat",
  relatedMessageId: "", threadRootId: "root", text: "スレッド本文", isPrompt: false, createdAtMs: Date.now() };
client.getReqseq = async () => 1;
client.square.sendMessage = async () => { ordinarySends++; throw new Error("UnexpectedOrdinarySend"); };
client.square.getSquareThreadMid = async options => gate.run("getSquareThreadMid", async () => {
  const request = options?.request;
  assert(request);
  assert.equal(marks, 0);
  assert.equal(request.chatMid, "chat");
  assert.equal(request.messageId, "root");
  lookups++;
  if (failLookup || lookups === 1) throw new Error("NOT_FOUND");
  return { threadMid: "thread" };
});
client.square.sendSquareThreadMessage = async options => gate.run("sendSquareThreadMessage", async () => {
  const request = options?.request;
  assert(request?.threadMessage);
  gate.beforeFetch(); sends++;
  assert.equal(marks, 1);
  assert.equal(request.chatMid, "chat");
  assert.equal(request.threadMid, "thread");
  assert.equal(request.threadMessage.message?.to, "thread");
  assert.equal(request.threadMessage.message?.toType, "SQUARE_THREAD");
  assert.equal(request.threadMessage.message?.text, "スレッド本文");
  if (failSend) throw new Error("TIMEOUT");
  return { createdThreadMessage: { message: { id: "sent-id" } } } as Awaited<ReturnType<typeof client.square.sendSquareThreadMessage>>;
});
try {
  const sent = await deliverAction(client, core, gate, action);
  assert.equal(sent.status, "sent");
  assert.equal(lookups, 2);
  assert.equal(sends, 1);
  assert.equal(results.at(-1)?.messageId, "sent-id");
  marks = 0; lookups = 0; failSend = true;
  assert.equal((await deliverAction(client, core, gate, action)).status, "unknown");
  assert.equal(sends, 2);
  assert.equal(results.at(-1)?.status, "unknown");
  marks = 0; lookups = 0; failLookup = true;
  assert.equal((await deliverAction(client, core, gate, action)).status, "failed");
  assert.equal(lookups, 3);
  assert.equal(sends, 2);
  assert.equal(marks, 0);
  assert.equal(ordinarySends, 0);
} finally { controller.abort(); }
console.log("skd adapter smoke passed");
