import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createCore, PROTOCOL_VERSION } from "./protocol/native.js";

// 文面変更・再起動・保存済み配送・Runtime間の独立性をLINEへ接続せず確認する。
const directory = await mkdtemp(join(tmpdir(), "kbc-messages-"));
const contentDirectory = join(directory, "content");
await cp("content", contentDirectory, { recursive: true });
const config = { databasePath: join(directory, "core.sqlite"), ownerId: "fixture", contentDirectory };
let core = createCore(config);
let sequence = 0;
async function submit(text: string) {
  const id = String(++sequence);
  await core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "fixture", checkpoint: id, baselineBeforeMs: null,
    events: [{ type: "messageReceived", eventId: id, chatId: "chat", messageId: id, text, createdAtMs: Date.now() }] });
}
async function take() {
  const action = await core.nextAction(); assert(action?.type === "sendMessage");
  core.markSending(action.actionId);
  core.completeAction({ actionId: action.actionId, status: "sent", code: "OK", messageId: `sent-${sequence}` });
  return action.text;
}
const path = join(contentDirectory, "messages/search.txt");
const original = await readFile(path, "utf8");
async function replace(key: string, body: string) {
  await writeFile(path, original.replace(new RegExp(`^${key.replaceAll(".", "\\.")} = .*$`, "m"), `${key} = ${JSON.stringify(body)}`));
}
try {
  await submit("!test-notify 60");
  await replace("search.prepare_04", "🙂 {seconds}秒待ってください。{{固定}}");
  core.shutdown(); core = createCore(config);
  assert.equal(await take(), "60秒後に通知を送ります。");
  await submit("o.test-notify 60");
  assert.equal(await take(), "🙂 60秒待ってください。{固定}");
  const other = createCore({ ...config, databasePath: join(directory, "other.sqlite"), contentDirectory: "content" });
  try {
    await other.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "other", checkpoint: "1", baselineBeforeMs: null,
      events: [{ type: "messageReceived", eventId: "other", chatId: "other", messageId: "1", text: "!test-notify 60", createdAtMs: Date.now() }] });
    const action = await other.nextAction(); assert(action?.type === "sendMessage" && action.text === "60秒後に通知を送ります。");
  } finally { other.shutdown(); }
  core.shutdown();
  for (const [body, code] of [[original + "\nunknown.key = test", "UnknownMessageKey"],
    [original + "\nsearch.prepare_04 = test", "DuplicateMessageKey"],
    [original.replace(/^search.prepare_04 = .*$/m, ""), "MissingMessageKey"]]) {
    await writeFile(path, body!); assert.throws(() => createCore(config), new RegExp(code!));
  }
  await replace("search.prepare_04", "{not_supplied}");
  assert.throws(() => createCore(config), /UnknownMessageVariable/);
  await replace("search.prepare_04", "{"); assert.throws(() => createCore(config), /InvalidMessageVariable/);
  // 引用符なしの1行も受け付ける。
  await writeFile(path, original.replace(/^search.apply_inner_02 = .*$/m, "search.apply_inner_02 = 結果なし: {arg0}"));
  core = createCore(config); await submit("!ut {not_supplied}"); assert.equal(await take(), "結果なし: ユニット");
  console.log(JSON.stringify({ ok: true, scenarios: ["custom-text-restart-outbox-runtime-isolation", "invalid-config-startup"] }));
} finally { core.shutdown(); }
