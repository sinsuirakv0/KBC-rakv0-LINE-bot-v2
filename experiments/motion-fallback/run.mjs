import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { createCore, PROTOCOL_VERSION } from '../../dist/protocol/native.js';

// 実LINE/Discordへ投稿せず、両NativeとHTTP境界・公開素材を使う任意の結合実験。
const require = createRequire(import.meta.url);
const discordRoot = resolve(process.env.DISCORD_BOT_ROOT || '../KBC-rakv0-discord-bot-v2');
const ffmpegPath = process.env.FFMPEG_PATH || require(join(discordRoot, 'node_modules/ffmpeg-static'));
const { createEventUpdateServer } = require(join(discordRoot, 'apps/discord/dist/external-events/server.js'));
const native = require(join(discordRoot, 'native/kbc_node.node'));
const remote = await native.createCore({ contentDirectory: join(discordRoot, 'content'), ffmpegPath });
const secret = randomBytes(32).toString('hex');
const requests = [];
const apiCore = {
  submitMotion: (request) => { requests.push(JSON.parse(request)); return remote.submitMotion(request); },
  motionStatus: (id) => remote.motionStatus(id),
  readMotionArtifact: (id) => remote.readMotionArtifact(id),
  removeMotion: (id) => remote.removeMotion(id),
};
const server = createEventUpdateServer({ motionSecret: secret, core: apiCore, isReady: () => true });
async function listen(server) {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return `http://127.0.0.1:${server.address().port}/motion-jobs`;
}
const endpoint = await listen(server);
const directory = await mkdtemp(join(tmpdir(), 'kbc-motion-fallback-'));
const config = { databasePath: join(directory, 'core.sqlite'), ownerId: 'fixture',
  contentDirectory: resolve('content'), searchDataPath: resolve('data/search/catalog.json'),
  motionRemoteUrl: endpoint, motionRemoteSecret: secret };
let core = createCore(config), worker, sequence = 0;
const results = [];
async function submit(text) {
  const messageId = String(++sequence);
  await core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: 'fixture', checkpoint: messageId,
    baselineBeforeMs: null, events: [{ type: 'messageReceived', eventId: `chat:${messageId}`, chatId: 'chat',
      messageId, text, senderId: 'owner', createdAtMs: Date.now() }] });
}
function sent(action) {
  core.markSending(action.actionId);
  core.completeAction({ actionId: action.actionId, status: 'sent', code: 'Fixture', messageId: `bot-${sequence}` });
}
async function take() {
  let timer;
  const action = await Promise.race([core.nextAction(), new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('MotionFallbackExperimentTimeout')), 90000);
  })]).finally(() => clearTimeout(timer));
  assert.equal(action?.type, 'sendMessage');
  return action;
}
async function artifact(name, frames, durationMs) {
  const action = await take(); assert(action.attachment, action.text);
  const bytes = await core.prepareAttachment(action.actionId);
  assert(bytes.length > 0 && bytes.length <= 8 * 1024 * 1024);
  const path = join(directory, name); await writeFile(path, bytes);
  const decoded = execFileSync(ffmpegPath, ['-v', 'error', '-i', path, '-fps_mode', 'passthrough', '-f', 'framemd5', '-'],
    { encoding: 'utf8', windowsHide: true });
  assert.equal(decoded.split('\n').filter((line) => line && !line.startsWith('#')).length, frames);
  if (durationMs !== undefined) assert.equal(action.attachment.durationMs, durationMs);
  results.push({ name, bytes: bytes.length, frames, durationMs: action.attachment.durationMs });
  sent(action);
}
async function post(value, token = secret) {
  return fetch(endpoint, { method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify(value) });
}
const headers = { authorization: `Bearer ${secret}` };
let oversized;
try {
  // 待機10分を過ぎたローカル生成中の強制終了を再現し、保存済みActionを代行へ再開する。
  await submit('!ut 0 motion png f a 0'); sent(await take());
  const db = new DatabaseSync(config.databasePath);
  assert.equal(db.prepare("UPDATE actions SET status='preparing',code='MotionLocalRunning',payload=json_set(payload,'$.createdAtMs',CAST(? AS INTEGER)) WHERE status='queued' AND json_extract(payload,'$.type')='prepareMedia'").run(Date.now() - 660000).changes, 1);
  db.close(); core.shutdown(); core = createCore(config); worker = core.runMediaJobs();
  await artifact('resumed.png', 1); assert.equal(requests.length, 1);

  // ローカル成功では代行APIを使わない。
  await submit('!ut 0 motion png f a 0'); sent(await take());
  await artifact('local.png', 1); assert.equal(requests.length, 1);

  // FFmpegがないLINEから実際のDiscord Rendererへ切替。生成中のpingも配送する。
  await submit('!ut 0 motion mp4 f w 0~~2 a 0~~2'); sent(await take());
  await submit('!ping'); const ping = await take(); assert.equal(ping.text, 'pong!'); sent(ping);
  await artifact('delegated.mp4', 6, 200);
  await submit('!tut 0 motion gif a 0~~2'); sent(await take());
  await artifact('delegated.gif', 3); assert.equal(requests.length, 3);

  // 不正なFrameは代行しない。
  await submit('!ut 0 motion png f a 999999'); sent(await take());
  const invalid = await take(); assert(!invalid.attachment); sent(invalid); assert.equal(requests.length, 3);

  const request = { ...requests[0], requestId: 'a'.repeat(40) };
  assert.equal((await post(request, 'wrong-secret')).status, 401);
  assert.equal((await post({ ...request, protocolVersion: 2 })).status, 400);
  assert.equal((await post({ ...request, plan: { ...request.plan, sprite_path: '../outside.png' } })).status, 400);
  assert.equal((await post(request)).status, 202);
  assert.equal((await post(request)).status, 202);
  assert.equal((await post({ ...request, plan: { ...request.plan, preview_scale: 1 } })).status, 409);
  const second = { ...request, requestId: 'b'.repeat(40) };
  assert.equal((await post(second)).status, 202);
  assert.equal((await post({ ...request, requestId: 'c'.repeat(40) })).status, 429);
  for (const value of [request, second]) {
    let status;
    for (let attempt = 0; attempt < 100; attempt++) {
      status = await (await fetch(`${endpoint}/${value.requestId}`, { headers })).json();
      if (status.status !== 'pending') break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.equal(status.status, 'ready');
    const download = await fetch(`${endpoint}/${value.requestId}/artifact`, { headers });
    assert.equal(download.status, 200); assert((await download.arrayBuffer()).byteLength > 0);
    assert.equal((await fetch(`${endpoint}/${value.requestId}`, { method: 'DELETE', headers })).status, 200);
    assert.equal((await fetch(`${endpoint}/${value.requestId}`, { headers })).status, 404);
  }

  // 受取側でも成果上限を検査し、失敗後に通常入力を継続する。
  core.shutdown(); await worker;
  oversized = createServer((request, response) => {
    if (request.method === 'POST') response.end(JSON.stringify({ protocolVersion: 1, status: 'accepted' }));
    else if (request.url.endsWith('/artifact')) {
      response.writeHead(200, { 'content-type': 'video/mp4', 'content-length': 8 * 1024 * 1024 + 1,
        'x-motion-protocol-version': '1', 'x-motion-file-name': 'ut-000-f-motion.mp4', 'x-motion-duration-ms': '100' });
      response.end();
    } else response.end(JSON.stringify({ protocolVersion: 1, status: 'ready' }));
  });
  const oversizedUrl = await listen(oversized);
  core = createCore({ ...config, motionRemoteUrl: oversizedUrl }); worker = core.runMediaJobs();
  await submit('!ut 0 motion mp4 f a 0~~2'); sent(await take());
  const failure = await take(); assert(!failure.attachment); sent(failure);
  await submit('!ping'); const afterFailure = await take(); assert.equal(afterFailure.text, 'pong!'); sent(afterFailure);
  console.log(JSON.stringify({ ok: true, lineNetwork: false, discordNetwork: false, publicAssetNetwork: true,
    recoveredAfterRestart: true, duplicateConflictAndCapacity: true, rejectedOversizedResult: true, directory, results }));
} finally {
  core.shutdown(); if (worker) await worker;
  await remote.shutdown();
  await new Promise((resolve) => server.close(resolve));
  if (oversized) await new Promise((resolve) => oversized.close(resolve));
}
