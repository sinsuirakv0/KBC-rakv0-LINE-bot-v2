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
    pages: 0, events: 0, messages: 0, duplicates: 0, pings: 0, chats: 0, authUpdated: false };
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

  function stop(reason) {
    state.reason ??= reason;
    controller.abort();
  }
  async function record(kind, data) {
    const line = JSON.stringify({ at: new Date().toISOString(), kind, ...data }) + "\n";
    logBytes += Buffer.byteLength(line);
    if (kind !== "finished" && logBytes > 5 * 1024 * 1024) throw failure("LogLimit");
    await appendFile(logPath, line, { mode: 0o600 });
    console.log(line.trimEnd());
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
      maxRequests, mode: "serialized-fetchMyEvents", sendEnabled: false, resources: await cgroupResources() });
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
      if (!["getProfile", "fetchMyEvents", "refresh"].includes(method)) throw failure("UnexpectedApi");
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
      if (events.length > 100) throw failure("UnexpectedBatchSize");
      for (const event of events) {
        if (state.events >= 10000) throw failure("EventLimit");
        const message = (event.payload?.notificationMessage ?? event.payload?.receiveMessage)?.squareMessage?.message;
        const entry = { sequence: ++state.events, type: String(event.type).slice(0, 80) };
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
          if (message.text === "o.ping") {
            entry.ping = true;
            if (!entry.duplicate) state.pings++;
          }
        }
        await record("event", entry);
      }
      await record("page", { page: ++state.pages, events: events.length, continuation: Boolean(page.continuationToken) });
      if (page.continuationToken && page.continuationToken === continuationToken && !events.length) {
        throw failure("ContinuationNoProgress");
      }
      syncToken = page.syncToken || syncToken;
      continuationToken = page.continuationToken || undefined;
      if (authDirty) { await storage.set(".auth", base.authToken); authDirty = false; }
      // checkpointは実験側だけに保存し、旧Botの再開位置を変更しない。
      await writeFile(join(outputDir, "checkpoint.json"), JSON.stringify({ syncToken, continuationToken }), { mode: 0o600 });
      await delay(continuationToken ? 1000 : intervalMs, undefined, { signal: controller.signal });
    }
  } catch (error) {
    if (!controller.signal.aborted) state.reason = errorCode(error);
  } finally {
    clearTimeout(deadline);
    controller.abort();
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
