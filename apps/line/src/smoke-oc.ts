import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { BaseClient } from "@evex/linejs/base";
import { ApiScheduler } from "./adapter/api.js";
import { deliverAction } from "./adapter/delivery.js";
import { SquareDirectory } from "./adapter/square.js";
import { normalizeEvent } from "./adapter/events.js";
import { createCore, PROTOCOL_VERSION, type CoreAction } from "./protocol/native.js";
import type { CoreEvent } from "./protocol/generated/CoreEvent.js";

// LINEへ接続せず、永続ActionとSDK境界の主要な運用経路を検証する。
const directory = await mkdtemp(join(tmpdir(), "kbc-oc-smoke-"));
const mid = (prefix: string, digit: string) => prefix + digit.repeat(32);
const square = mid("s", "1"), chat = mid("m", "1"), sub = mid("m", "2");
const admin = mid("p", "1"), co = mid("p", "2"), mod = mid("p", "3"), user = mid("p", "4"), bot = mid("p", "5");
const owner = mid("p", "9");
const roles = new Map([[admin, "ADMIN"], [co, "CO_ADMIN"], [mod, "MEMBER"], [user, "MEMBER"], [bot, "CO_ADMIN"]]);
const states = new Map<string, string>();
await writeFile(join(directory, "permissions.json"), JSON.stringify({ version: 1, roles: [
  { chatMid: square, userMid: mod, chatType: "SQUARE", role: "mod" },
  { chatMid: square, userMid: owner, chatType: "SQUARE", role: "admin" },
] }));
const config = { databasePath: join(directory, "core.sqlite"), ownerId: "fixture-account", permissionsPath: join(directory, "permissions.json") };
let core = createCore(config);
const db = new DatabaseSync(config.databasePath);
const controller = new AbortController(), gate = new ApiScheduler(controller.signal, 2, 1);
const client = new BaseClient({ device: "DESKTOPWIN" });
const member = (id: string) => ({ squareMemberMid: id, squareMid: square, displayName: id === user ? "参加者🙂" : "メンバー",
  role: roles.get(id) ?? "MEMBER", membershipState: states.get(id) ?? "JOINED", revision: 7n });
let failMutation = false;
client.square.getSquareChat = async ({ squareChatMid }) => ({ squareChat: { squareChatMid, squareMid: square, name: "検証", type: "SQUARE_DEFAULT" },
  squareChatMember: { squareMemberMid: bot } }) as Awaited<ReturnType<typeof client.square.getSquareChat>>;
client.square.getSquareMember = async ({ squareMemberMid }) => ({ squareMember: member(squareMemberMid) }) as Awaited<ReturnType<typeof client.square.getSquareMember>>;
client.square.searchSquareMembers = async () => ({ members: [member(user)] }) as Awaited<ReturnType<typeof client.square.searchSquareMembers>>;
client.square.updateSquareMember = async options => gate.run("updateSquareMember", async () => {
  const updated = options?.request?.squareMember; assert(updated?.squareMemberMid);
  gate.beforeFetch(); assert.equal(core.stats().sendingActions, 1);
  if (failMutation) throw new Error("DisconnectedAfterRequest");
  states.set(updated.squareMemberMid, String(updated.membershipState));
  return { squareMember: member(updated.squareMemberMid) } as Awaited<ReturnType<typeof client.square.updateSquareMember>>;
});
const deleted: string[] = [];
client.square.destroyMessage = async options => gate.run("destroyMessage", async () => {
  gate.beforeFetch(); deleted.push(options.messageId); return {} as Awaited<ReturnType<typeof client.square.destroyMessage>>;
});
let promptSequence = 0;
const replySends: Array<{ chat: string; message: string; text: string | undefined }> = [];
client.square.sendMessage = async options => gate.run("sendMessage", async () => {
  if (options.relatedMessageId) replySends.push({ chat: options.squareChatMid, message: options.relatedMessageId, text: options.text });
  gate.beforeFetch(); return { createdSquareMessage: { message: { id: `bot-${++promptSequence}` } } } as Awaited<ReturnType<typeof client.square.sendMessage>>;
});
const service = new SquareDirectory(client);
let sequence = 0;
async function submitEvent(event: CoreEvent) {
  return core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "account", checkpoint: String(++sequence), baselineBeforeMs: null, events: [event] });
}
async function message(text: string, actor = admin, replyToMessageId?: string, targetChat = chat, extra: Partial<Extract<CoreEvent, { type: "messageReceived" }>> = {}) {
  const id = `message-${sequence}`;
  await submitEvent({ type: "messageReceived", eventId: `${targetChat}:${id}`, chatId: targetChat, messageId: id, senderId: actor,
    squareId: square, botMemberId: bot, text, replyToMessageId, createdAtMs: Date.now(), ...extra });
  return id;
}
const actions: CoreAction[] = [];
async function drain() {
  const start = actions.length;
  for (let count = 0; count < 100; count++) {
    const next = db.prepare("SELECT payload FROM actions WHERE status='queued' AND due<=? ORDER BY due,rowid LIMIT 1").get(Date.now() + 100) as { payload: string } | undefined;
    if (!next) return actions.slice(start);
    const data = JSON.parse(next.payload) as CoreAction;
    const action = await (data.type === "ocApi" && ["context", "member", "chats", "members", "joinedChats"].includes(data.request.type) ? core.nextQueryAction() : core.nextAction());
    assert(action); actions.push(action);
    await deliverAction(client, core, gate, action, service);
  }
  throw new Error("FixtureDidNotDrain");
}
const texts = (values: CoreAction[]) => values.filter(a => a.type === "sendMessage").map(a => a.text).join("\n");
const settings = () => JSON.parse((db.prepare("SELECT payload FROM oc_settings WHERE square=?").get(square) as { payload: string }).payload);
async function signal(id: string, state: string, scope = "square", targetChat = chat, at = Date.now()) {
  return submitEvent({ type: "memberChanged", eventId: `member-${sequence}`, squareId: square, chatId: targetChat, memberId: id,
    displayName: "参加者🙂", state, scope, memberCreatedAtMs: at, createdAtMs: at });
}
try {
  // 返信元が別トーク・別OCでも送信先は実行トーク。BOT管理者だけが本文を崩さず1件送信できる。
  const replyMessage = "123456789012345678";
  await message("返信元の投稿", user, undefined, sub, { messageId: replyMessage }); await drain();
  for (const actor of [admin, mod]) {
    await message(`!test reply ${replyMessage} 未許可`, actor);
    assert(texts(await drain()).includes("BOT管理者専用"));
    assert.equal(replySends.length, 0);
  }
  const replyText = "返信テスト🙂\n次の行  二つの空白";
  await message(`!test reply ${replyMessage} ${replyText}`, owner);
  assert.equal((await drain()).filter(action => action.type === "sendMessage").length, 1);
  assert.deepEqual(replySends.at(-1), { chat, message: replyMessage, text: replyText });
  await message(`o.test reply ${replyMessage} --chat ${sub} サブトークの投稿へ返信`, owner); await drain();
  assert.deepEqual(replySends.at(-1), { chat, message: replyMessage, text: "サブトークの投稿へ返信" });
  const otherChat = mid("m", "a");
  const otherMessage = "123456789012345679";
  await message("別OCの投稿", user, undefined, otherChat, { messageId: otherMessage, squareId: mid("s", "a") }); await drain();
  await message(`!test reply ${otherMessage} --chat ${otherChat} 別OCの投稿へ返信`, owner); await drain();
  assert.deepEqual(replySends.at(-1), { chat, message: otherMessage, text: "別OCの投稿へ返信" });
  await message(`!test reply 123456789012345680 --chat ${otherChat} 未観測の投稿の表示確認`, owner); await drain();
  assert.equal(replySends.at(-1)?.chat, chat);
  const beforeInvalid = replySends.length;
  await message(`!test reply ${chat} MIDを誤指定`, owner); assert(texts(await drain()).includes("メッセージID"));
  await message(`!test reply ${replyMessage} --chat ${square} OCのMIDを誤指定`, owner); assert(texts(await drain()).includes("トークMID"));
  await message(`!test reply ${replyMessage} --chat ${otherChat} 返信元MIDの不一致`, owner); assert(texts(await drain()).includes("一致しません"));
  await message(`!test reply ${replyMessage} --to ${sub} 旧引数で別トークへ送らない`, owner); assert(texts(await drain()).includes("--chat"));
  await message(`!test reply ${replyMessage} ${"🙂".repeat(751)}`, owner); assert(texts(await drain()).includes("1,500"));
  assert.equal(replySends.length, beforeInvalid);
  await message(`!test reply ${replyMessage} -- --chatから始まる本文`, owner); await drain();
  assert.equal(replySends.at(-1)?.text, "--chatから始まる本文");
  await message("!test reply help", user);
  const testHelp = await drain(); assert(texts(testHelp).includes("BOT管理者専用"));
  assert(testHelp.every(action => action.type === "sendMessage"));
  // 旧kicktestは案内だけ。通常応答は返信先を付けず、サブトークの受信済みIDを参照する。
  await message(`!oc kicktest ${user}`, mod); const kickTest = await drain();
  assert(!kickTest.some(action => action.type === "ocApi" && action.request.type === "membership"));
  assert(texts(kickTest).includes("confirmは不要"));
  await message("!id", user); const selfId = await drain(); assert(texts(selfId).includes(user));
  assert(selfId.filter(action => action.type === "sendMessage").every(action => action.relatedMessageId === ""));
  const sourceId = await message("別トークの発言", user, undefined, sub, { senderName: "参加者🙂" }); await drain();
  await message("!id reply", user, sourceId); const reference = await drain();
  assert(texts(reference).includes(`relatedMessageId: ${sourceId}`) && texts(reference).includes(`元トークMID: ${sub}`));
  await message(`!id message ${sourceId} --chat ${sub}`, user); assert(texts(await drain()).includes("参加者🙂"));
  await message("!id 参加者", user); assert(texts(await drain()).includes(user));
  // 旧権限区分、本人への番号返信、照会待ちの通常配送と再起動。
  await message(`o.oc kick ${user}`, admin); assert(texts(await drain()).includes("実行権限"));
  await message("o.oc setup", mod); assert(texts(await drain()).includes("実行権限"));
  await message(`o.oc mute ${user} inf`, co); assert(texts(await drain()).includes("実行権限"));
  await message("!oc setup", co); await drain(); const oldPrompt = `bot-${promptSequence}`;
  assert.equal((await message("1", user, oldPrompt), await drain()).length, 0);
  await message("1 2", co, oldPrompt); await drain(); assert(settings().url && settings().media);
  await message("8", co, `bot-${promptSequence}`); await drain(); assert(!settings().url && !settings().media);
  await message("o.oc media on", co); const held = await core.nextQueryAction(); assert(held?.type === "ocApi");
  await message("o.ping", user); const ping = await core.nextAction(); assert(ping?.type === "sendMessage" && ping.text === "pong!");
  await deliverAction(client, core, gate, ping, service); assert.equal(core.stats().queryingActions, 1);
  core.shutdown(); core = createCore(config); await drain(); assert(settings().media);
  await message(`o.oc kick ${user}`, mod); failMutation = true; const unknown = await drain(); failMutation = false;
  assert(unknown.some(a => a.type === "ocApi" && a.request.type === "membership")); assert.equal(core.stats().unknownActions, 1);
  core.shutdown(); core = createCore(config); assert.equal(core.stats().unknownActions, 1);
  await message("!oc kick history", mod); const kickHistory = texts(await drain());
  assert(kickHistory.includes("強制退会・再参加禁止履歴") && kickHistory.includes("結果不明"));
  assert(kickHistory.includes("参加者🙂") && kickHistory.includes(mod));

  // 明示muteは権限免除より優先。URL免除と連投の7件目を確認。
  await message(`o.oc mute ${user} inf`, mod); await drain();
  roles.set(user, "ADMIN"); const muted = await message("o.ping", user); const warning = await drain(); assert(deleted.includes(muted));
  assert(warning.some(action => action.type === "sendMessage" && action.mention && action.text.includes("無期限")));
  assert(db.prepare("SELECT 1 FROM actions WHERE id LIKE '%:oc:mute-notice:0:cleanup' AND due>?").get(Date.now()));
  roles.set(user, "MEMBER"); await message(`o.oc mute ${user} off`, mod); await drain();
  await message("o.oc url add https://example.com/docs prefix"); await drain(); await message("o.oc url on"); await drain();
  const prefixed = await message("o.ping", user); assert(texts(await drain()).includes("pong!")); assert(!deleted.includes(prefixed));
  await message("!ping", user); assert(texts(await drain()).includes("pong!"));
  const allowed = await message("https://example.com/docs/page", user); assert.equal((await drain()).length, 0); assert(!deleted.includes(allowed));
  const blocked = await message("https://example.com/docs-evil", user); await drain(); assert(deleted.includes(blocked));
  const attached = await message("こちらhttps://blocked.example/", user); await drain(); assert(deleted.includes(attached));
  const longUrl = await message("https://blocked.example/" + "x".repeat(15000), user); await drain(); assert(deleted.includes(longUrl));
  const exempt = await message("https://other.example/", co); await drain(); assert(!deleted.includes(exempt));
  const pictures: string[] = [];
  for (let i = 1; i <= 7; i++) { pictures.push(await message("", user, undefined, chat, { contentType: "IMAGE", mediaGroupSequence: i, mediaGroupTotal: 7 })); await drain(); }
  assert(pictures.slice(0, 6).every(id => !deleted.includes(id))); assert(deleted.includes(pictures[6]));

  // PUSHの正規化、トーク通知、サブトーク退出とOC全体退出を区別。
  await message("o.oc main set"); await drain(); await message("o.oc watch early on"); await drain();
  await message("o.oc join set --mention --id <name>さん\nようこそ"); await drain();
  const joined = mid("p", "6"); roles.set(joined, "MEMBER"); const joinAt = Date.now();
  await signal(joined, "JOINED", "square", chat, joinAt); const notified = await drain();
  const notification = notified.find(a => a.type === "sendMessage" && a.mention); assert(notification?.type === "sendMessage");
  assert(notification.text.includes("参加者🙂さん\nようこそ") && notification.text.includes("ID:")); assert.equal(notification.relatedMessageId, "");
  await signal(joined, "LEFT", "chat", sub, joinAt + 1); assert.equal((await drain()).length, 0); assert.equal(states.get(joined), undefined);
  states.set(joined, "LEFT"); await signal(joined, "LEFT", "square", chat, joinAt + 2); await drain(); assert.equal(states.get(joined), "BANNED");
  await message("o.oc watch danger on"); await drain(); await message("o.oc watch cohort on"); await drain();
  const newcomer = mid("p", "7"); await signal(newcomer, "JOINED"); await drain();
  await message("ﾁｰﾄの代行", newcomer); await drain(); assert.equal(states.get(newcomer), "KICK_OUT");
  await message("o.oc modroom set"); await drain();
  await message("!oc leftmessage set <name>さん退出"); await drain();
  const returning = mid("p", "b"); const returnAt = Date.now();
  await signal(returning, "JOINED", "square", chat, returnAt); await drain();
  await signal(returning, "BANNED", "square", chat, returnAt + 1); assert.equal((await drain()).length, 0);
  await signal(returning, "JOINED", "square", chat, returnAt + 2); await drain(); states.set(returning, "LEFT");
  await signal(returning, "LEFT", "square", chat, returnAt + 3); const returningLeft = await drain();
  assert(texts(returningLeft).includes("再参加者"));
  assert(!returningLeft.some(action => action.type === "ocApi" && action.request.type === "membership"));
  for (const digit of ["8", "9", "a"]) { await signal(mid("p", digit), "JOINED"); await drain(); }
  const cohortMember = mid("p", "a"); await message("招待はこちら https://example.com/docs/", cohortMember); const watched = await drain();
  assert(texts(watched).includes("自動処分なし")); assert.equal(states.get(cohortMember), undefined);
  await message("https://allow.example/path", user); const review = await drain();
  assert(texts(review).includes("完全一致"));
  const storedCase = db.prepare("SELECT prompt FROM oc_cases WHERE url LIKE 'https://allow.example/%'").get() as { prompt: string };
  assert(storedCase.prompt);
  await message("4", co, storedCase.prompt); await drain();
  const approved = await message("https://allow.example/another", user); await drain(); assert(!deleted.includes(approved));
  const converted = await normalizeEvent({ type: "NOTIFIED_CREATE_SQUARE_MEMBER", createdTime: BigInt(Date.now()), payload: {
    notifiedCreateSquareMember: { squareMember: { ...member(user), createdAt: BigInt(Date.now()) } } } } as Parameters<typeof normalizeEvent>[0], service);
  assert(converted?.type === "memberChanged" && converted.scope === "square" && converted.state === "JOINED");
  const media = await normalizeEvent({ type: "RECEIVE_MESSAGE", payload: { receiveMessage: { squareMessage: { message: {
    id: "media-fixture", to: chat, from: user, createdTime: BigInt(Date.now()), contentType: "IMAGE", contentMetadata: { GSEQ: "999999999999", GTOTAL: "7" } } } } } } as unknown as Parameters<typeof normalizeEvent>[0], service);
  assert(media?.type === "messageReceived" && media.text === "" && media.mediaGroupSequence === undefined);
  console.log(JSON.stringify({ ok: true, scenarios: ["permissions-session-query-restart-unknown", "mute-url-media", "push-notification-oc-leave"] }));
} finally { controller.abort(); core.shutdown(); db.close(); }
