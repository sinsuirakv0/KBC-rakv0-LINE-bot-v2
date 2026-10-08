import { SquareDirectory } from "./adapter/square.js";
import { BaseClient, type Device } from "@evex/linejs/base";
import { resolve } from "node:path";
import { createServer } from "node:http";
import { AuthStorage } from "./adapter/storage.js";
import { GitHubPersistence } from "./adapter/persistence.js";
import { LogSync } from "./adapter/logs.js";
import { SearchDataUpdater } from "./adapter/search-data.js";
import { ApiScheduler, errorCode, installApiScheduler } from "./adapter/api.js";
import { Receiver } from "./adapter/receiver.js";
import { deliverAction } from "./adapter/delivery.js";
import { createCore, type NativeCore } from "./protocol/native.js";
import { RuntimeMonitor } from "./adapter/runtime.js";

const QUERY_WORKERS = 2;
const DELIVERY_WORKERS = 2;

function integerSetting(name: string, fallback: number, min: number, max: number): number {
  const value = Number(process.env[name] ?? fallback);
  if (!Number.isInteger(value) || value < min || value > max) throw new Error(`Invalid${name}`);
  return value;
}

async function main(): Promise<void> {
  if (process.env.LINE_OLD_BOT_STOPPED !== "1") throw new Error("StopOldBotBeforeStarting");
  const device = process.env.LINE_DEVICE ?? "DESKTOPWIN";
  if (!["DESKTOPWIN", "DESKTOPMAC", "ANDROID", "ANDROIDSECONDARY", "IOS", "IOSIPAD", "WATCHOS", "WEAROS"].includes(device)) throw new Error("InvalidDevice");
  const controller = new AbortController();
  const authPath = resolve(process.env.LINE_STORAGE_FILE ?? "storage/auth.json");
  const databasePath = resolve(process.env.CORE_DATABASE_PATH ?? "storage/core.sqlite");
  const searchDataPath = resolve(process.env.SEARCH_DATA_PATH ?? "storage/search/catalog.json");
  const searchData = new SearchDataUpdater(searchDataPath);
  const persistence = GitHubPersistence.fromEnvironment(authPath, databasePath);
  const restoredFromBackup = await persistence?.restore() ?? false;
  const permissionsPath = process.env.BOT_PERMISSIONS_PATH?.trim() ? resolve(process.env.BOT_PERMISSIONS_PATH) :
    persistence ? resolve("storage/permissions.json") : undefined;
  const legacyOcSettingsPath = process.env.LEGACY_OC_SETTINGS_PATH?.trim() ? resolve(process.env.LEGACY_OC_SETTINGS_PATH) :
    persistence ? resolve("storage/legacy-oc-settings.json") : undefined;
  if (persistence && permissionsPath && legacyOcSettingsPath) await persistence.restoreSettings(permissionsPath, legacyOcSettingsPath);
  const storage = new AuthStorage(authPath, error => controller.abort(error),
    persistence ? data => persistence.protectAuth(data) : undefined);
  await storage.load();
  const token = await storage.get(".auth") ?? process.env.LINE_AUTH_TOKEN;
  if (typeof token !== "string" || !token) throw new Error("MissingStoredAuthToken");
  const gate = new ApiScheduler(controller.signal, integerSetting("LINE_API_CONCURRENCY", 2, 1, 4),
    integerSetting("LINE_API_INTERVAL_MS", 250, 100, 5000));
  const client = new BaseClient({ device: device as Device, storage });
  client.config.timeout = 15000;
  installApiScheduler(client, gate, controller.signal);
  client.on("update:authtoken", token => {
    client.authToken = token;
    void storage.set(".auth", token).catch(() => {});
  });
  let core: NativeCore | undefined;
  let receiver: Receiver | undefined;
  let logs: LogSync | undefined;
  const monitor = new RuntimeMonitor(gate, QUERY_WORKERS, DELIVERY_WORKERS,
    () => ({ state: receiver?.status ?? "starting", chats: receiver?.metrics.listedChats ?? 0 }));
  await monitor.initialize();
  let started = false;
  let shutdownTimer: NodeJS.Timeout | undefined;
  const deliveries = { sent: 0, failed: 0, unknown: 0, queued: 0, maxQueueWaitMs: 0 };
  const stop = () => controller.abort();
  controller.signal.addEventListener("abort", () => {
    core?.shutdown();
    shutdownTimer = setTimeout(() => process.exit(1), 20000);
    shutdownTimer.unref();
  }, { once: true });
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  const server = createServer((request, response) => {
    const healthy = started && !controller.signal.aborted && receiver?.status === "receiving";
    response.writeHead(request.url === "/health" ? (healthy ? 200 : 503) : 404, { "Content-Type": "application/json" });
    response.end(JSON.stringify({ state: receiver?.status ?? "starting", core: core?.stats(), api: gate.metrics,
      receiver: receiver?.metrics, deliveries, backup: persistence?.metrics,
      logs: logs?.metrics, searchData: searchData.metrics,
      runtime: monitor.snapshot(), rssBytes: process.memoryUsage().rss, uptimeSeconds: process.uptime() }));
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(integerSetting("PORT", 3000, 1, 65535), "0.0.0.0", resolve);
  });
  const metricsTimer = setInterval(() => {
    void monitor.refresh().then(() => {
      console.log(JSON.stringify({ kind: "metrics", state: receiver?.status, core: core?.stats(), api: gate.metrics,
        receiver: receiver?.metrics, deliveries, runtime: monitor.snapshot(),
        cpuCorePercent: monitor.snapshot().cpuCorePercent, searchData: searchData.metrics, memory: process.memoryUsage() }));
    }).catch(error => console.error(JSON.stringify({ kind: "runtimeMetricsFailure", code: errorCode(error) })));
  }, 60000);
  const tasks: Promise<void>[] = [];
  try {
    await client.loginProcess.login({ authToken: token });
    await storage.flush();
    if (!client.profile?.mid) throw new Error("MissingAccountOwner");
    core = createCore({ databasePath, ownerId: client.profile.mid, restoredFromBackup,
      logsEnabled: Boolean(persistence) && process.env.OC_LOGS_ENABLED === "1",
      maxRetainedEvents: integerSetting("CORE_MAX_RETAINED_EVENTS", 131072, 8192, 524288),
      contentDirectory: resolve(process.env.CONTENT_DIRECTORY ?? "content"),
      searchDataPath, searchDataLive: true,
      permissionsPath, legacyOcSettingsPath,
      ffmpegPath: process.env.FFMPEG_PATH?.trim() || (process.platform === "linux" ? "/usr/bin/ffmpeg" : undefined),
      motionRemoteUrl: process.env.MOTION_REMOTE_URL?.trim() || undefined,
      motionRemoteSecret: process.env.MOTION_REMOTE_SECRET?.trim() || undefined }, () => monitor.snapshot());
    const directory = new SquareDirectory(client);
    receiver = new Receiver(client, core, gate, controller.signal, directory);
    const activeCore = core;
    if (persistence && process.env.OC_LOGS_ENABLED === "1") logs = new LogSync(activeCore, persistence);
    await persistence?.backup(activeCore);
    const deliver = async (query = false) => {
      while (!controller.signal.aborted) {
        const action = await (query ? activeCore.nextQueryAction() : activeCore.nextAction());
        if (!action) return;
        const queuedMs = Date.now() - action.createdAtMs;
        if (action.type === "sendMessage") deliveries.maxQueueWaitMs = Math.max(deliveries.maxQueueWaitMs, queuedMs);
        const { status, code } = await deliverAction(client, activeCore, gate, action, directory);
        deliveries[status]++;
        console.log(JSON.stringify({ kind: "delivery", operation: action.type, status, code, queueWaitMs: queuedMs }));
      }
    };
    started = true;
    tasks.push(receiver.run(), ...Array.from({ length: DELIVERY_WORKERS }, () => deliver()),
      ...Array.from({ length: QUERY_WORKERS }, () => deliver(true)), activeCore.runMediaJobs(), activeCore.runStoreMonitors());
    tasks.push(searchData.run(controller.signal));
    if (persistence) tasks.push(persistence.run(activeCore, controller.signal));
    if (logs) tasks.push(logs.run(controller.signal));
    await Promise.all(tasks);
  } finally {
    stop();
    core?.shutdown();
    await Promise.allSettled(tasks);
    try {
      await storage.flush();
      try { await logs?.flush(); }
      catch (error) { console.error(JSON.stringify({kind: "logSyncFailure", code: errorCode(error)})); }
      if (core) await persistence?.backup(core);
    }
    finally {
      clearInterval(metricsTimer);
      server.closeAllConnections();
      await new Promise<void>(resolve => server.close(() => resolve()));
      if (shutdownTimer) clearTimeout(shutdownTimer);
      process.removeListener("SIGINT", stop);
      process.removeListener("SIGTERM", stop);
    }
  }
}

void main().then(() => process.exit(0)).catch(error => {
  console.error(JSON.stringify({ kind: "fatal", code: errorCode(error) }));
  process.exit(1);
});
