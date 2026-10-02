import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { createCore, PROTOCOL_VERSION } from "../../dist/protocol/native.js";

// 公開ゲーム素材を取得する任意の結合実験。LINE認証・LINE通信は行わない。
const ffmpegPath = process.env.FFMPEG_PATH;
assert(ffmpegPath, "Set FFMPEG_PATH to run PNG/MP4/GIF verification");
const ffprobe = join(dirname(ffmpegPath), process.platform === "win32" ? "ffprobe.exe" : "ffprobe");
const directory = await mkdtemp(join(tmpdir(), "kbc-media-check-"));
const core = createCore({ databasePath: join(directory, "core.sqlite"), ownerId: "fixture",
  contentDirectory: resolve("content"), searchDataPath: resolve("data/search/catalog.json"), ffmpegPath });
const worker = core.runMediaJobs();
let sequence = 0, timerTicks = 0;
const timer = setInterval(() => timerTicks++, 5);
async function submit(text, replyToMessageId) {
  const messageId = String(++sequence);
  await core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: "fixture", checkpoint: messageId, baselineBeforeMs: null,
    events: [{ type: "messageReceived", eventId: `chat:${messageId}`, chatId: "chat", messageId, text, senderId: "owner", replyToMessageId, createdAtMs: Date.now() }] });
}
function sent(action, messageId = `bot-${sequence}`) {
  core.markSending(action.actionId); core.completeAction({ actionId: action.actionId, status: "sent", code: "Fixture", messageId });
}
async function take() {
  while (true) {
    let timeout;
    const action = await Promise.race([core.nextAction(), new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error("MediaExperimentTimeout")), 90000); })]).finally(() => clearTimeout(timeout));
    assert(action);
    if (action.type === "deleteMessage") { sent(action); continue; }
    assert.equal(action.type, "sendMessage"); return action;
  }
}
const outputs = [];
async function attachment(name) {
  const action = await take(); assert(action.attachment, action.text);
  const bytes = await core.prepareAttachment(action.actionId); assert(bytes.length > 0 && bytes.length <= 8 * 1024 * 1024);
  const output = join(directory, name); await writeFile(output, bytes);
  const probe = JSON.parse(execFileSync(ffprobe, ["-v", "error", "-count_frames", "-show_entries", "stream=codec_name,width,height,nb_read_frames:format=duration", "-of", "json", output], { encoding: "utf8", windowsHide: true }));
  const stream = probe.streams[0]; assert(stream.width > 0 && stream.height > 0);
  outputs.push({ name, bytes: bytes.length, kind: action.attachment.kind, durationMs: action.attachment.durationMs, probe });
  if (action.attachment.kind === "video") assert.equal(Math.round(Number(probe.format.duration) * 1000), action.attachment.durationMs);
  sent(action); return action;
}
try {
  for (const [command, name] of [
    ["o.ut 0 motion png f a 0", "unit.png"], ["o.ut 0 motion mp4 f w 0~~5 a 0~~5", "unit.mp4"],
    ["o.ut 0 motion gif f a 0~~3", "unit.gif"], ["o.tut 0 motion png a 0", "enemy.png"],
  ]) {
    await submit(command); const acknowledgment = await take(); assert(acknowledgment.text.includes("受け付けました")); sent(acknowledgment);
    if (name === "unit.mp4") {
      const started = performance.now(); await submit("o.ping"); const ping = await take(); assert.equal(ping.text, "pong!");
      outputs.push({ pingDuringGenerationMs: performance.now() - started }); sent(ping);
    }
    await attachment(name);
  }
  await submit("o.ut 0 file f"); sent(await take());
  const files = await take(); assert(files.isPrompt && files.text.includes("アイコン")); sent(files, "files-prompt");
  await submit("1", "files-prompt"); sent(await take()); await attachment("icon.png");
  assert(timerTicks > 0);
  console.log(JSON.stringify({ ok: true, lineNetwork: false, publicAssetNetwork: true, timerTicks, directory, outputs }));
} finally { clearInterval(timer); core.shutdown(); await worker; }
