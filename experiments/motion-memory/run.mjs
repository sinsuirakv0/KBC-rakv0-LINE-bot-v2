import { createCore, PROTOCOL_VERSION } from '../../dist/protocol/native.js';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

// 実運用で報告された素材を、LINE通信なしで同じRendererへ渡す。
const format = process.env.MOTION_FORMAT ?? 'mp4';
if (!['png', 'mp4', 'gif'].includes(format)) throw Error('InvalidFormat');
const baselineMiB = Number(process.env.MOTION_BASELINE_MIB ?? 110);
if (![110, 420].includes(baselineMiB)) throw Error('InvalidBaseline');
const baseline = Buffer.alloc(baselineMiB * 1024 * 1024, 1);
const directory = await mkdtemp(join(tmpdir(), 'kbc-motion-memory-'));
const core = createCore({ databasePath: join(directory, 'core.sqlite'), ownerId: 'fixture',
  contentDirectory: resolve('content'), searchDataPath: resolve('data/search/catalog.json'), ffmpegPath: '/usr/bin/ffmpeg' });
const worker = core.runMediaJobs();
const command = `!ut 710 motion ${format} f ${format === 'png' ? 'a 0' : 'w i a k'}`;
const started = Date.now();
await core.submitBatchAsync({ protocolVersion: PROTOCOL_VERSION, streamKey: 'fixture', checkpoint: '1',
  events: [{ type: 'messageReceived', eventId: 'fixture', messageId: 'fixture', chatId: 'fixture',
    senderId: 'fixture', text: command, createdAtMs: started }] });
let result;
try {
  for (let index = 0; index < 3; index++) {
    const action = await core.nextAction();
    if (!action) break;
    core.markSending(action.actionId);
    core.completeAction({ actionId: action.actionId, status: 'sent', code: 'Fixture', messageId: `fixture-${index}` });
    if (action.attachment) { result = action.attachment; break; }
    if (!action.text.includes('受け付けました')) { result = { error: action.text }; break; }
  }
} finally { core.shutdown(); await worker; }
let peakBytes;
try { peakBytes = Number(await readFile('/sys/fs/cgroup/memory.peak', 'utf8')); }
catch { try { peakBytes = Number(await readFile('/sys/fs/cgroup/memory/memory.max_usage_in_bytes', 'utf8')); } catch {} }
console.log(JSON.stringify({ command, result, baselineBytes: baseline.length, elapsedMs: Date.now() - started,
  rssBytes: process.memoryUsage().rss, parentMaxRssBytes: process.resourceUsage().maxRSS * 1024, cgroupPeakBytes: peakBytes }));
if (!result || result.error) process.exitCode = 1;
