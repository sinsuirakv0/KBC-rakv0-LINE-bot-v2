import { createHmac, randomBytes } from "node:crypto";
import { appendFile, copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { resolve, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { BaseClient } from "@evex/linejs/base";
import { FileStorage } from "@evex/linejs/storage";

const sdk = JSON.parse(await readFile(new URL("./node_modules/@evex/linejs/package.json", import.meta.url)));
if (sdk.version !== "3.4.2") throw new Error("Unexpected SDK version");

function numberSetting(name, fallback, min, max) {
  const value = Number(process.env[name] ?? fallback);
  if (!Number.isInteger(value) || value < min || value > max) throw new Error(`Invalid ${name}`);
  return value;
}

async function cgroupResources() {
  const result = {};
  for (const [name, file] of Object.entries({
    cpuMax: "/sys/fs/cgroup/cpu.max", memoryMax: "/sys/fs/cgroup/memory.max",
    memoryCurrent: "/sys/fs/cgroup/memory.current", cpuStat: "/sys/fs/cgroup/cpu.stat",
  })) {
    try { result[name] = (await readFile(file, "utf8")).trim(); }
    catch { result[name] = null; }
  }
  return result;
}

// エラー本文には認証情報等が含まれ得るため、限定したcodeだけを記録する。
function errorCode(error) {
  const code = error?.code ?? error?.data?.code ?? error?.cause?.code ?? error?.type ?? error?.name;
  return /^[A-Za-z0-9_-]{1,80}$/.test(String(code)) ? String(code) : "UNKNOWN";
}

function failure(code) {
  return Object.assign(new Error(code), { code });
}

if (process.argv.includes("--check")) {
  console.log(JSON.stringify({ sdk: sdk.version, node: process.version, network: false,
    deviceConfigured: Boolean(process.env.LINE_DEVICE), resources: await cgroupResources() }));
} else {
  if (!process.argv.includes("--live") || !process.argv.includes("--old-receiver-stopped")) {
    throw new Error("Use --check or --live --old-receiver-stopped");
  }
  await runLiveProbe();
}

async function runLiveProbe() {
  const durationMs = numberSetting("PROBE_DURATION_SECONDS", 300, 30, 900) * 1000;
  const intervalMs = numberSetting("PROBE_INTERVAL_MS", 1000, 1000, 30000);
  const maxRequests = numberSetting("PROBE_MAX_REQUESTS", 400, 10, 1000);
  const replyEnabled = process.env.PROBE_REPLY_ENABLED === "1";
  const compareChat = process.env.PROBE_COMPARE_CHAT === "1";
  const experimentStartedAt = Date.now();
  const sourcePath = resolve(process.env.LINE_STORAGE_FILE || "./storage/storage.json");
  const device = process.env.LINE_DEVICE;
  if (!device) throw new Error("LINE_DEVICE must match the old container");
  const runId = process.env.PROBE_RUN_ID;
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(runId ?? "")) throw new Error("PROBE_RUN_ID is required");
  const outputDir = resolve(process.env.PROBE_OUTPUT_DIR || `./logs/receiver-probe-${runId}`);
  if (sourcePath === join(outputDir, "auth.json")) throw new Error("Auth copy must be separate");
  await mkdir(outputDir, { recursive: true, mode: 0o700 });
  const logPath = join(outputDir, "observations.ndjson");
  const salt = randomBytes(32);
  const hashId = (value) => createHmac("sha256", salt).update(String(value)).digest("hex").slice(0, 24);
  const controller = new AbortController();
  const state = { status: "preparing", reason: null, requests: 0, httpRequests: 0,
    pages: 0, events: 0, messages: 0, duplicates: 0, pings: 0, chats: 0, authUpdated: false,
    repliesQueued: 0, repliesSent: 0, chatPages: 0 };
  const seen = new Set();
  const chats = new Set();
  const methods = {};
  let logBytes = 0;
  let base;
  let authDirty = false;
  let lastAuthToken;
  let storage;
  let syncToken;
  let continuationToken;
  let testChatMid;
  let chatSyncToken;
  let chatPending = false;
  const replyQueue = [];
  let replyWake;
  let replyTask;

  function stop(reason) {
    state.reason ??= reason;
    controller.abort();
    replyWake?.();
  }
  async function record(kind, data) {
    const line = JSON.stringify({ at: new Date().toISOString(), kind, runId, ...data }) + "\n";
    logBytes += Buffer.byteLength(line);
    if (kind !== "finished" && logBytes > 5 * 1024 * 1024) throw failure("LogLimit");
    await appendFile(logPath, line, { mode: 0o600 });
    console.log(line.trimEnd());
  }

  async function recordEvents(events, source, pageNumber) {
    if (events.length > 100) throw failure("UnexpectedBatchSize");
    for (const event of events) {
      if (state.events >= 10000) throw failure("EventLimit");
      const message = (event.payload?.notificationMessage ?? event.payload?.receiveMessage ?? event.payload?.sendMessage)?.squareMessage?.message;
      const entry = { sequence: ++state.events, type: String(event.type).slice(0, 80), source, page: pageNumber };
      let shouldReply = false;
      if (message?.id && message?.to) {
        const key = hashId(`${message.to}:${message.id}`);
        entry.chat = hashId(message.to);
        entry.message = key;
        entry.duplicate = seen.has(key);
        if (entry.duplicate) state.duplicates++;
        else { seen.add(key); state.messages++; }
        chats.add(entry.chat);
        state.chats = chats.size;
        const createdAt = Number(message.createdTime);
        if (Number.isFinite(createdAt) && createdAt > 0) entry.lagMs = Date.now() - createdAt;
        const pingMatch = typeof message.text === "string" ? /^o\.ping(?:\s+([AB]\d{2}))?$/u.exec(message.text.trim()) : null;
        if (pingMatch) {
          entry.ping = true;
          entry.pingTag = pingMatch[1];
          if (!entry.duplicate) state.pings++;
          // 番号付き新着で試験OCを確定し、他OCや過去入力へ返信しない。
          if (createdAt >= experimentStartedAt) {
            if (!testChatMid && ["A01", "B01"].includes(entry.pingTag)) testChatMid = message.to;
            shouldReply = replyEnabled && message.to === testChatMid && !entry.duplicate;
          }
        }
        if (compareChat && source === "notification" && message.to === testChatMid) chatPending = true;
      }
      await record("event", entry);
      if (shouldReply) {
        if (state.repliesQueued >= 50 || replyQueue.length >= 32) throw failure("ReplyLimit");
        replyQueue.push({ chat: message.to, message: message.id, tag: entry.pingTag });
        state.repliesQueued++;
        replyWake?.();
      }
    }
  }

  async function sendReplies() {
    while (!controller.signal.aborted) {
      if (!replyQueue.length) {
        await new Promise((done) => { replyWake = done; });
        replyWake = undefined;
        continue;
      }
      const reply = replyQueue.shift();
      await base.square.sendMessage({ squareChatMid: reply.chat, relatedMessageId: reply.message,
        text: reply.tag ? `pong ${reply.tag}` : "pong" });
      state.repliesSent++;
      await record("reply", { chat: hashId(reply.chat), message: hashId(`${reply.chat}:${reply.message}`),
        pingTag: reply.tag, status: "sent" });
      await delay(1000, undefined, { signal: controller.signal });
    }
  }

  // 実験終了後はLINE通信せず待機し、正常終了からの再起動ループを避ける。
  const server = createServer((request, response) => {
    response.writeHead(request.url === "/health" ? 200 : 404, { "Content-Type": "application/json" });
    response.end(JSON.stringify({ experiment: true, ...state }));
  });
  await new Promise((done, fail) => {
    server.once("error", fail);
    server.listen(numberSetting("PORT", 3000, 1, 65535), "0.0.0.0", done);
  });
  const deadline = setTimeout(() => stop("duration"), durationMs);
  const terminate = () => stop("signal");
  process.once("SIGTERM", terminate);
  process.once("SIGINT", terminate);
  let metricsTask;
  try {
    // 同じRun IDの再実行を防ぐ。コンテナ交換時の永続性は配備前に確認する。
    await writeFile(join(outputDir, "started"), new Date().toISOString(), { flag: "wx", mode: 0o600 });
    await copyFile(sourcePath, join(outputDir, "auth.json"));
    storage = new FileStorage(join(outputDir, "auth.json"));
    const storedToken = await storage.get(".auth");
    const token = typeof storedToken === "string" && storedToken.trim() ? storedToken : process.env.LINE_AUTH_TOKEN;
    if (!token) throw failure("NoStoredAuthToken");
    await record("start", { sdk: sdk.version, node: process.version, durationMs, intervalMs,
      maxRequests, mode: "serialized-fetchMyEvents", sendEnabled: replyEnabled, compareChat, resources: await cgroupResources() });
    base = new BaseClient({ device, storage });
    const nativeFetch = base.fetch;
    // 標準Node transportを保持し、全HTTP通信に終了signalと回数上限を付ける。
    base.fetch = async (input, init) => {
      controller.signal.throwIfAborted();
      if (state.httpRequests >= maxRequests) throw failure("HttpRequestLimit");
      state.httpRequests++;
      const request = new Request(input, init);
      const response = await nativeFetch(new Request(request, {
        signal: AbortSignal.any([request.signal, controller.signal]),
      }));
      if (!response.ok) {
        const retryAfter = response.headers.get("retry-after");
        await record("http-error", { status: response.status,
          retryAfterSeconds: retryAfter && /^\d{1,8}$/.test(retryAfter) ? Number(retryAfter) : null });
        stop(`http-${response.status}`);
        await response.body?.cancel();
        throw failure("HttpError");
      }
      return response;
    };
    const requestCore = base.request.requestCore.bind(base.request);
    base.request.requestCore = async (...args) => {
      controller.signal.throwIfAborted();
      const method = args[2];
      if (!["getProfile", "fetchMyEvents", "refresh", ...(compareChat ? ["fetchSquareChatEvents"] : []),
        ...(replyEnabled ? ["sendMessage"] : [])].includes(method)) throw failure("UnexpectedApi");
      if (state.requests >= maxRequests) throw failure("RequestLimit");
      state.requests++;
      methods[method] = (methods[method] ?? 0) + 1;
      const startedAt = Date.now();
      try {
        const result = await requestCore(...args);
        await record("api", { method, elapsedMs: Date.now() - startedAt, ok: true });
        return result;
      } catch (error) {
        await record("api", { method, elapsedMs: Date.now() - startedAt, ok: false, code: errorCode(error) });
        throw error;
      }
    };
    base.on("update:authtoken", (nextToken) => {
      base.authToken = nextToken;
      authDirty = true;
      if (lastAuthToken !== undefined && lastAuthToken !== nextToken) state.authUpdated = true;
      lastAuthToken = nextToken;
    });
    await base.loginProcess.login({ authToken: token });
    if (authDirty) await storage.set(".auth", base.authToken);
    authDirty = false;
    const owner = await storage.get("kbc.squareSyncOwnerMid");
    if (owner !== base.profile.mid) throw failure("CheckpointOwnerMismatch");
    syncToken = await storage.get("kbc.squareSyncToken");
    if (typeof syncToken !== "string" || !syncToken) throw failure("NoStoredCheckpoint");
    state.status = "receiving";
    await record("ready", { checkpointLoaded: true, ownerConfirmed: true });
    if (replyEnabled) replyTask = sendReplies().catch((error) => {
      if (!controller.signal.aborted) stop(errorCode(error));
    });
    metricsTask = (async () => {
      let cpu = process.cpuUsage();
      let at = Date.now();
      while (!controller.signal.aborted) {
        await delay(15000, undefined, { signal: controller.signal });
        const elapsedMs = Date.now() - at;
        const usage = process.cpuUsage(cpu);
        const cpuPercentOfOneCore = (usage.user + usage.system) / (elapsedMs * 10);
        await record("resources", { elapsedMs, cpuPercentOfOneCore, rssBytes: process.memoryUsage().rss,
          resources: await cgroupResources(), ...state });
        cpu = process.cpuUsage();
        at = Date.now();
      }
    })().catch((error) => {
      if (!controller.signal.aborted) stop(errorCode(error));
    });
    while (!controller.signal.aborted) {
      const page = await base.square.fetchMyEvents({ syncToken, continuationToken, limit: 100 });
      const events = page.events ?? [];
      await recordEvents(events, "notification", state.pages + 1);
      await record("page", { page: ++state.pages, events: events.length, continuation: Boolean(page.continuationToken) });
      if (page.continuationToken && page.continuationToken === continuationToken && !events.length) {
        throw failure("ContinuationNoProgress");
      }
      syncToken = page.syncToken || syncToken;
      continuationToken = page.continuationToken || undefined;
      if (authDirty) { await storage.set(".auth", base.authToken); authDirty = false; }
      // checkpointは実験側だけに保存し、旧Botの再開位置を変更しない。
      await writeFile(join(outputDir, "checkpoint.json"), JSON.stringify({ syncToken, continuationToken }), { mode: 0o600 });
      if (compareChat && testChatMid && chatPending) {
        if (state.chatPages >= 40) throw failure("ChatCompareLimit");
        const chatPage = await base.square.fetchSquareChatEvents({ squareChatMid: testChatMid, syncToken: chatSyncToken, limit: 100, direction: "FORWARD" });
        await recordEvents(chatPage.events ?? [], "chat", state.chatPages + 1);
        chatSyncToken = chatPage.syncToken || chatSyncToken;
        chatPending = Boolean(chatPage.events?.length);
        await record("chat-page", { page: ++state.chatPages, events: chatPage.events?.length ?? 0 });
      }
      await delay(continuationToken ? 1000 : intervalMs, undefined, { signal: controller.signal });
    }
  } catch (error) {
    if (!controller.signal.aborted) state.reason = errorCode(error);
  } finally {
    clearTimeout(deadline);
    controller.abort();
    replyWake?.();
    await replyTask;
    if (base) {
      base.disabled = true;
      if (authDirty) await storage.set(".auth", base.authToken);
    }
    await metricsTask;
    state.status = "finished";
    await record("finished", { ...state, methods, resources: await cgroupResources() });
    await writeFile(join(outputDir, "summary.json"), JSON.stringify({ ...state, methods }, null, 2));
    process.removeListener("SIGTERM", terminate);
    process.removeListener("SIGINT", terminate);
  }
}
