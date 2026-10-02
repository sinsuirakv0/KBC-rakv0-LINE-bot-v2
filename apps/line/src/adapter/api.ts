import { AsyncLocalStorage } from "node:async_hooks";
import { setTimeout as delay } from "node:timers/promises";
import type { BaseClient } from "@evex/linejs/base";

export function errorCode(error: unknown): string {
  const value = error as { code?: unknown; data?: { code?: unknown; errorCode?: unknown }; name?: string; message?: string } | null;
  const safeMessage = /^[A-Za-z0-9_-]{1,80}$/.test(value?.message ?? "") ? value?.message : undefined;
  const code = String(value?.data?.errorCode ?? value?.data?.code ?? (value?.code === "GenericFailure" ? undefined : value?.code) ?? safeMessage ?? value?.name ?? "Unknown");
  return /^[A-Za-z0-9_-]{1,80}$/.test(code) ? code : "Unknown";
}

type Job = { execute: () => Promise<unknown>; resolve: (value: unknown) => void; reject: (error: unknown) => void };
export type SendAttempt = { started: boolean; method?: "sendMessage" | "destroyMessage" | "uploadMedia"; beforeSend: () => void };

export class ApiScheduler {
  private queue: Job[] = [];
  private active = 0;
  private nextStart = 0;
  private cooldownUntil = 0;
  private scope = new AsyncLocalStorage<boolean>();
  private methodScope = new AsyncLocalStorage<string>();
  private sendScope = new AsyncLocalStorage<SendAttempt>();
  readonly metrics = { requests: 0, errors: 0, rateLimits: 0, maxActive: 0, totalWaitMs: 0, totalApiMs: 0, methods: {} as Record<string, number> };

  constructor(private signal: AbortSignal, private concurrency = 2, private intervalMs = 250) {
    signal.addEventListener("abort", () => {
      for (const job of this.queue.splice(0)) job.reject(new Error("Stopping"));
    }, { once: true });
  }

  async run<T>(method: string, operation: () => Promise<T>): Promise<T> {
    this.signal.throwIfAborted();
    const queuedAt = Date.now();
    const execute = async () => {
      await this.pace();
      const started = Date.now();
      this.metrics.totalWaitMs += started - queuedAt;
      this.metrics.requests++;
      const key = /^[A-Za-z0-9_]{1,60}$/.test(method) ? method : "other";
      if (key in this.metrics.methods || Object.keys(this.metrics.methods).length < 32) {
        this.metrics.methods[key] = (this.metrics.methods[key] ?? 0) + 1;
      }
      try { return await this.methodScope.run(method, operation); }
      catch (error) {
        this.metrics.errors++;
        this.cooldownFor(error);
        throw error;
      } finally { this.metrics.totalApiMs += Date.now() - started; }
    };
    // SDKのtoken更新・再要求は親RPCの枠内で順に動くため、二重に枠を取らない。
    if (this.scope.getStore()) return execute();
    if (this.queue.length >= 32) throw Object.assign(new Error("ApiQueueFull"), { code: "ApiQueueFull" });
    return new Promise<T>((resolve, reject) => {
      this.queue.push({ execute, resolve: value => resolve(value as T), reject });
      this.pump();
    });
  }

  withSendAttempt<T>(attempt: SendAttempt, operation: () => Promise<T>): Promise<T> {
    return this.sendScope.run(attempt, operation);
  }

  beforeFetch(): void {
    const attempt = this.sendScope.getStore();
    if (!attempt || this.methodScope.getStore() !== (attempt.method ?? "sendMessage") || attempt.started) return;
    attempt.beforeSend();
    attempt.started = true;
  }

  async checkUploadResponse(response: Response): Promise<void> {
    // SDKのOBS uploadはHTTP statusを検査しないので、共通transportで補う。
    if (["uploadImage", "uploadMedia"].includes(this.methodScope.getStore() ?? "") && !response.ok) {
      await response.body?.cancel();
      throw Object.assign(new Error(`Http${response.status}`), { code: String(response.status) });
    }
  }

  async pace(): Promise<void> {
    const reserved = Math.max(Date.now(), this.nextStart, this.cooldownUntil);
    this.nextStart = reserved + this.intervalMs;
    let start = reserved;
    while (true) {
      if (start < this.cooldownUntil) {
        start = Math.max(this.nextStart, this.cooldownUntil);
        this.nextStart = start + this.intervalMs;
      }
      const wait = start - Date.now();
      if (wait <= 0) break;
      await delay(wait, undefined, { signal: this.signal });
    }
    this.signal.throwIfAborted();
  }

  cooldown(seconds = 60): void {
    this.metrics.rateLimits++;
    this.cooldownUntil = Math.max(this.cooldownUntil, Date.now() + Math.min(600, Math.max(1, seconds)) * 1000);
  }

  cooldownFor(error: unknown): void {
    if ((error as { rateLimitHandled?: boolean } | null)?.rateLimitHandled) return;
    if (/LIMIT|TOO_MANY|EXHAUSTED|EXCESSIVE_ACCESS|^429$/i.test(errorCode(error))) this.cooldown();
  }

  private pump(): void {
    while (!this.signal.aborted && this.active < this.concurrency && this.queue.length) {
      const job = this.queue.shift()!;
      this.active++;
      this.metrics.maxActive = Math.max(this.metrics.maxActive, this.active);
      void this.scope.run(true, job.execute).then(job.resolve, job.reject).finally(() => { this.active--; this.pump(); });
    }
  }
}

async function checkRateLimit(response: Response, gate: ApiScheduler): Promise<Response> {
  if (response.status !== 429) return response;
  const retryAfter = Number(response.headers.get("retry-after"));
  gate.cooldown(Number.isFinite(retryAfter) && retryAfter > 0 ? retryAfter : 60);
  await response.body?.cancel();
  throw Object.assign(new Error("HttpRateLimit"), { code: "429", rateLimitHandled: true });
}

export function installApiScheduler(client: BaseClient, gate: ApiScheduler, signal: AbortSignal): void {
  // 3.4.2配布物の内部境界。LEGY・通常RPCとも、本文の処理完了まで枠を保持する。
  const request = client.request as unknown as { requestCore: (...args: unknown[]) => Promise<unknown> };
  if (typeof request.requestCore !== "function") throw new Error("UnsupportedLinejsRequest");
  const original = request.requestCore.bind(client.request);
  request.requestCore = (...args) => gate.run(String(args[2]), () => original(...args));
  const nativeFetch = client.fetch;
  const transport = client as unknown as { fetch: typeof client.fetch; fetchPush: typeof client.fetchPush };
  transport.fetch = async (input, init) => {
    const request = new Request(input, init);
    signal.throwIfAborted();
    request.signal.throwIfAborted();
    // reqseq保存・API待機・Thrift/LEGYの準備が終わった実transportの直前。
    gate.beforeFetch();
    const response = await checkRateLimit(await nativeFetch(new Request(request, {
      signal: AbortSignal.any([request.signal, signal, AbortSignal.timeout(15000)]),
    })), gate);
    await gate.checkUploadResponse(response);
    return response;
  };
  // Node標準のHTTP/2 transportは保持し、PUSH接続をRPC枠へ混ぜない。
  const nativePush = client.fetchPush;
  transport.fetchPush = async (input, init) => {
    const request = new Request(input, init);
    return checkRateLimit(await nativePush(new Request(request, { signal: AbortSignal.any([request.signal, signal]) })), gate);
  };
}
