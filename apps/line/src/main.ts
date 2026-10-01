import { BaseClient, type Device } from "@evex/linejs/base";
import { resolve } from "node:path";
import { createServer } from "node:http";
import { AuthStorage } from "./adapter/storage.js";
import { ApiScheduler, errorCode, installApiScheduler } from "./adapter/api.js";
import { Receiver } from "./adapter/receiver.js";
import { createCore, type NativeCore } from "./protocol/native.js";

function integerSetting(name: string, fallback: number, min: number, max: number): number {
  const value = Number(process.env[name] ?? fallback);
  if (!Number.isInteger(value) || value < min || value > max) throw new Error(`Invalid${name}`);
  return value;
}

async function main(): Promise<void> {
  if (process.env.LINE_OLD_BOT_STOPPED !== "1") throw new Error("StopOldBotBeforeStarting");
  const device = process.env.LINE_DEVICE ?? "DESKTOPWIN";
  if (!["DESKTOPWIN", "DESKTOPMAC", "ANDROID", "ANDROIDSECONDARY", "IOS", "IOSIPAD", "WATCHOS", "WEAROS"].includes(device)) throw new Error("InvalidDevice");
  const storage = new AuthStorage(resolve(process.env.LINE_STORAGE_FILE ?? "storage/auth.json"));
  await storage.load();
  const token = await storage.get(".auth") ?? process.env.LINE_AUTH_TOKEN;
  if (typeof token !== "string" || !token) throw new Error("MissingStoredAuthToken");
  const controller = new AbortController();
  const gate = new ApiScheduler(controller.signal, integerSetting("LINE_API_CONCURRENCY", 2, 1, 4),
    integerSetting("LINE_API_INTERVAL_MS", 250, 100, 5000));
  const client = new BaseClient({ device: device as Device, storage });
  client.config.timeout = 15000;
  installApiScheduler(client, gate, controller.signal);
  let authTask = Promise.resolve();
  client.on("update:authtoken", token => {
    client.authToken = token;
    authTask = authTask.then(() => storage.set(".auth", token));
    void authTask.catch(() => controller.abort());
  });
  let core: NativeCore | undefined;
  let receiver: Receiver | undefined;
  let started = false;
  let shutdownTimer: NodeJS.Timeout | undefined;
  const deliveries = { sent: 0, unknown: 0, failed: 0, maxQueueWaitMs: 0 };
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
      receiver: receiver?.metrics, deliveries, rssBytes: process.memoryUsage().rss, uptimeSeconds: process.uptime() }));
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(integerSetting("PORT", 3000, 1, 65535), "0.0.0.0", resolve);
  });
  const cpuStart = process.cpuUsage();
  const wallStart = Date.now();
  const metricsTimer = setInterval(() => {
    const cpu = process.cpuUsage(cpuStart);
    console.log(JSON.stringify({ kind: "metrics", state: receiver?.status, core: core?.stats(), api: gate.metrics,
      receiver: receiver?.metrics, deliveries, cpuCorePercent: (cpu.user + cpu.system) / ((Date.now() - wallStart) * 10),
      memory: process.memoryUsage() }));
  }, 60000);
  const tasks: Promise<void>[] = [];
  try {
    await client.loginProcess.login({ authToken: token });
    await authTask;
    if (!client.profile?.mid) throw new Error("MissingAccountOwner");
    core = createCore({ databasePath: resolve(process.env.CORE_DATABASE_PATH ?? "storage/core.sqlite"), ownerId: client.profile.mid });
    receiver = new Receiver(client, core, gate, controller.signal);
    const activeCore = core;
    const deliver = async () => {
      while (!controller.signal.aborted) {
        const action = await activeCore.nextAction();
        if (!action) return;
        let status: "sent" | "unknown" = "sent";
        let code = "OK";
        const queuedMs = Date.now() - action.createdAtMs;
        deliveries.maxQueueWaitMs = Math.max(deliveries.maxQueueWaitMs, queuedMs);
        try {
          await client.square.sendMessage({ squareChatMid: action.chatId, relatedMessageId: action.relatedMessageId, text: action.text });
        } catch (error) { status = "unknown"; code = errorCode(error); }
        activeCore.completeAction({ actionId: action.actionId, status, code });
        deliveries[status]++;
        console.log(JSON.stringify({ kind: "delivery", status, code, queueWaitMs: queuedMs }));
      }
    };
    started = true;
    tasks.push(receiver.run(), deliver(), deliver());
    await Promise.all(tasks);
  } finally {
    stop();
    core?.shutdown();
    await Promise.allSettled(tasks);
    await authTask;
    clearInterval(metricsTimer);
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    if (shutdownTimer) clearTimeout(shutdownTimer);
    process.removeListener("SIGINT", stop);
    process.removeListener("SIGTERM", stop);
  }
}

void main().then(() => process.exit(0)).catch(error => {
  console.error(JSON.stringify({ kind: "fatal", code: errorCode(error) }));
  process.exit(1);
});
