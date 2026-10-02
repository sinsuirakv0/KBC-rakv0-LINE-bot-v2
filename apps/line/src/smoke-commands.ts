import assert from "node:assert/strict";
import { cp, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { BaseClient } from "@evex/linejs/base";
import { ApiScheduler } from "./adapter/api.js";
import { deliverAction } from "./adapter/delivery.js";
import { createCore, PROTOCOL_VERSION, type CoreAction } from "./protocol/native.js";

const directory = await mkdtemp(join(tmpdir(), "kbc-command-smoke-"));
await cp("content", join(directory, "content"), { recursive: true });
await writeFile(join(directory, "content/responses/sample.txt"), "\ufeff追加した応答\r\n");
await writeFile(join(directory, "content/help/sample.txt"), "\ufeff追加した案内\r\n");
const config = { databasePath: join(directory, "core.sqlite"), ownerId: "fixture-account",
  contentDirectory: join(directory, "content"), searchDataPath: resolve("data/search/catalog.json") };
let core = createCore(config);
const db = new DatabaseSync(config.databasePath);
let sequence = 0;
async function submit(text: string, senderId = "alice", replyToMessageId?: string, chatId = "chat") {
  const messageId = String(++sequence);
  return core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "account", checkpoint: messageId, baselineBeforeMs: null,
    events: [{ type: "messageReceived", eventId: `${chatId}:${messageId}`, chatId, messageId, text, senderId, replyToMessageId, createdAtMs: Date.now() }] });
}
const deleted: string[] = [];
async function take(): Promise<Extract<CoreAction, { type: "sendMessage" }>> {
  while (true) {
    const action = await core.nextAction(); assert(action);
    if (action.type === "deleteMessage") { deleted.push(action.messageId); sent(action); continue; }
    assert(action.text.length <= 1500 && !action.text.includes("```"));
    return action;
  }
}
function sent(action: CoreAction, messageId = `bot-${sequence}`): void {
  core.markSending(action.actionId);
  core.completeAction({ actionId: action.actionId, status: "sent", code: "OK", messageId });
}

try {
  // txt追加だけで応答と案内・一覧が登録される。BOMとCRLFは出力へ持ち込まない。
  await submit("o.sample"); let action = await take(); assert.equal(action.text, "追加した応答"); sent(action);
  await submit("o.sample help"); action = await take(); assert.equal(action.text, "追加した案内"); sent(action);
  await submit("o.help"); action = await take(); assert(action.text.includes("o.sample") && action.text.includes("o.ut")); sent(action);
  for (const [command, expected] of [["o.unit ０", "ネコ"], ["o.tut 0", "わんこ"], ["o.st N000-000", "大地を揺るがす"], ["o.st 3000-000", "長崎県"]]) {
    await submit(command); action = await take(); assert(action.text.includes(expected) && action.text.includes("https://jarjarblink.github.io/JDB/"), action.text); sent(action);
  }

  // 本人・トーク・送信済みpromptに結び付け、再起動とページ更新後も誤選択しない。
  await submit("o.ut ねこ"); const firstPage = await take(); assert(firstPage.text.includes("9：次へ")); sent(firstPage, "prompt-one");
  assert.equal((await submit("1", "bob", "prompt-one")).actionsCreated, 0);
  assert.equal((await submit("1", "alice", "prompt-one", "other-chat")).actionsCreated, 0);
  assert.equal((await submit("1")).actionsCreated, 0);
  core.shutdown(); core = createCore(config);
  await submit("9", "alice", "prompt-one"); const secondPage = await take(); assert(secondPage.text.includes("9〜16") && secondPage.text.includes("0：前へ"));
  const oldCleanupId = `cleanup:${JSON.stringify(["chat", "prompt-one"])}`;
  assert((db.prepare("SELECT due FROM actions WHERE id=?").get(oldCleanupId) as { due: number }).due > Date.now());
  sent(secondPage, "prompt-two");
  assert.equal((await submit("1", "alice", "prompt-one")).actionsCreated, 0);
  await submit("1", "alice", "prompt-two"); action = await take(); assert(action.text.includes("https://jarjarblink.github.io/JDB/")); sent(action);
  assert(deleted.includes("prompt-one"));
  assert.equal(core.stats().activeSessions, 0);
  assert.equal((await submit("1", "alice", "prompt-two")).actionsCreated, 0);
  await submit("o.ut ねこ"); action = await take(); sent(action, "expired-prompt");
  db.prepare("UPDATE sessions SET expires=0").run();
  assert.equal((await submit("1", "alice", "expired-prompt")).actionsCreated, 0);
  const expiredCleanupId = `cleanup:${JSON.stringify(["chat", "expired-prompt"])}`;
  db.prepare("UPDATE actions SET due=0 WHERE id=?").run(expiredCleanupId);
  const expiredCleanup = await core.nextAction(); assert(expiredCleanup?.type === "deleteMessage" && expiredCleanup.messageId === "expired-prompt"); sent(expiredCleanup);

  // 画像取得に失敗した場合は送信前に通常返信へ切り替え、結果不明を増やさない。
  await submit("o.ut 0 origin c"); action = await take(); sent(action);
  const image = await take(); assert(image.imageUrl?.endsWith("uni000_c00.png"));
  db.prepare("UPDATE actions SET payload=? WHERE id=?").run(JSON.stringify({ ...image, imageUrl: "https://invalid.example/image.png" }), image.actionId);
  assert.equal(await core.prepareImage(image.actionId), null);
  const fallback = await take(); assert.equal(fallback.actionId, image.actionId); assert(!fallback.imageUrl && fallback.text.includes("画像を取得できません")); sent(fallback);

  // Adapterは実送信IDをCoreへ渡し、OBS失敗を送信済みと誤認しない。通信は模擬する。
  await submit("o.tut 0 origin"); action = await take(); sent(action); const enemyImage = await take();
  const controller = new AbortController(); const gate = new ApiScheduler(controller.signal, 2, 1);
  const client = new BaseClient({ device: "DESKTOPWIN" });
  let sdkSends = 0;
  client.square.sendMessage = async () => gate.run("sendMessage", async () => {
    gate.beforeFetch(); assert.equal(core.stats().sendingActions, 1);
    return { createdSquareMessage: { message: { id: sdkSends++ === 0 ? "image-message" : "prompt-message" } } } as Awaited<ReturnType<typeof client.square.sendMessage>>;
  });
  client.obs.uploadObjTalk = async (_chat, _type, blob, id) => {
    assert.equal(id, "image-message"); assert(blob.size > 0);
    await gate.checkUploadResponse(new Response(null, { status: 403 }));
    throw new Error("UploadStatusWasIgnored");
  };
  const result = await deliverAction(client, { ...core, prepareImage: async () => Buffer.from("fixture") }, gate, enemyImage);
  assert.equal(result.status, "unknown"); assert.equal(result.code, "403"); assert.equal(core.stats().unknownActions, 1);
  await submit("o.ut ねこ"); const prompt = await take();
  assert.equal((await deliverAction(client, core, gate, prompt)).status, "sent");
  await submit("1", "alice", "prompt-message"); action = await take(); assert(action.text.includes("https://jarjarblink.github.io/JDB/")); sent(action);
  const cleanup = await core.nextAction(); assert(cleanup?.type === "deleteMessage" && cleanup.messageId === "prompt-message");
  client.square.destroyMessage = async options => gate.run("destroyMessage", async () => {
    gate.beforeFetch(); assert.equal(options.messageId, "prompt-message"); throw new Error("AdminDeleteDenied");
  });
  await submit("o.ut ねこ");
  assert.equal((await deliverAction(client, core, gate, cleanup)).status, "unknown");
  assert.equal(core.stats().activeSessions, 1);
  action = await take(); sent(action, "after-delete-failure");
  await submit("1", "alice", "after-delete-failure"); action = await take(); assert(action.text.includes("https://")); sent(action);
  controller.abort();
  console.log(JSON.stringify({ ok: true, scenarios: ["txt-registration-and-real-data", "owned-reply-restart-pagination-expiry", "image-preparation-fallback", "image-upload-result"] }));
} finally { core.shutdown(); db.close(); }
