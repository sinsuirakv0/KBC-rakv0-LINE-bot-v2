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
let mediaWorker: Promise<void> | undefined;
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
    assert(action.type === "sendMessage");
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
  await submit("o.help"); action = await take(); assert(action.text.includes("!sample") && action.text.includes("!ut") && action.text.includes("!bot")); sent(action);
  await submit("!bot help"); action = await take(); assert(action.text.includes("!bot name") && action.text.includes("BOT管理者")); sent(action);
  for (const [command, expected] of [["o.unit 0", "ネコ"], ["o.st N0", "id=0"], ["o.tut 0", "わんこ"], ["o.st N000-000", "大地を揺るがす"], ["o.st 3000-000", "長崎県"]]) {
    await submit(command); action = await take(); assert(action.text.includes(expected) && action.text.includes("https://jarjarblink.github.io/JDB/"), action.text); sent(action);
  }

  await submit("o.ut ﾈｺﾋﾞﾙﾀﾞｰ"); action = await take();
  assert(action.text.includes("第二形態名でヒット") && action.text.includes("000 ネコ")); sent(action, "normalized-prompt");

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

  // 永続Mediaは8件まで。生成待ちの同じトークでも通常返信を塞がない。
  for (let index = 0; index < 8; index++) {
    await submit("o.ut 0 origin c"); action = await take(); sent(action);
  }
  assert.equal((await submit("o.ut 0 origin c")).actionsCreated, 1);
  action = await take(); assert(action.text.includes("混み合っています") && !action.text.includes("受け付け")); sent(action);
  const pending = db.prepare("SELECT id,payload FROM actions WHERE json_valid(payload) AND json_extract(payload,'$.type')='prepareMedia'").all() as { id: string; payload: string }[];
  assert.equal(pending.length, 8);
  for (const [index, row] of pending.entries()) {
    const job = JSON.parse(row.payload), request = JSON.parse(job.request);
    db.prepare("UPDATE actions SET payload=?,status='preparing' WHERE id=?").run(JSON.stringify({ ...job, request: JSON.stringify({ ...request, catalog_revision: index === 0 ? "old-snapshot" : request.catalog_revision, request: { Download: { path: "../invalid.png" } } }) }), row.id);
  }
  core.shutdown(); core = createCore(config);
  await submit("o.ping"); action = await take(); assert.equal(action.text, "pong!"); sent(action);
  mediaWorker = core.runMediaJobs();
  await assert.rejects(core.runMediaJobs(), /MediaWorkerAlreadyRunning/);
  for (let index = 0; index < 8; index++) {
    const fallback = await take(); assert(!fallback.attachment && fallback.text.includes(index === 0 ? "検索データが更新" : "失敗")); sent(fallback);
  }

  // 保存済み成果が失われても、LINE通信前に案内へ変えて他の返信を維持する。
  await submit("o.ping"); action = await take();
  db.prepare("UPDATE actions SET payload=? WHERE id=?").run(JSON.stringify({ ...action, attachment: { kind: "image", fileName: "missing.png", contentType: "image/png" } }), action.actionId);
  assert.equal(await core.prepareAttachment(action.actionId), null);
  action = await take(); assert(!action.attachment && action.text.includes("もう一度")); sent(action);

  // OCはOBS upload自身が投稿する。oid省略、動画の実時間、通信後の失敗を検証する。
  await submit("o.ping"); action = await take();
  const attachment = { kind: "video", fileName: "motion.mp4", contentType: "video/mp4", durationMs: 500 };
  db.prepare("UPDATE actions SET payload=? WHERE id=?").run(JSON.stringify({ ...action, text: "", attachment }), action.actionId);
  const mediaAction = { ...action, text: "", attachment };
  const controller = new AbortController(); const gate = new ApiScheduler(controller.signal, 2, 1);
  const client = new BaseClient({ device: "DESKTOPWIN" });
  let sdkSends = 0;
  client.square.sendMessage = async () => gate.run("sendMessage", async () => {
    gate.beforeFetch(); assert.equal(core.stats().sendingActions, 1); sdkSends++;
    return { createdSquareMessage: { message: { id: "prompt-message" } } } as Awaited<ReturnType<typeof client.square.sendMessage>>;
  });
  client.obs.uploadObjTalk = async (_chat, type, blob, id, name, duration) => {
    gate.beforeFetch(); assert.equal(core.stats().sendingActions, 1);
    assert.equal(type, "video"); assert.equal(id, undefined); assert.equal(name, "motion.mp4"); assert.equal(duration, 500); assert(blob.size > 0);
    await gate.checkUploadResponse(new Response(null, { status: 403 }));
    throw new Error("UploadStatusWasIgnored");
  };
  const result = await deliverAction(client, { ...core, prepareAttachment: async () => Buffer.from("fixture") }, gate, mediaAction);
  assert.equal(result.status, "unknown"); assert.equal(result.code, "403"); assert.equal(core.stats().unknownActions, 1); assert.equal(sdkSends, 0);
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
  console.log(JSON.stringify({ ok: true, scenarios: ["txt-and-discord-data", "owned-reply-restart-pagination-expiry", "media-job-recovery-and-error", "oc-upload-and-admin-delete"] }));
} finally { core.shutdown(); await mediaWorker; db.close(); }
