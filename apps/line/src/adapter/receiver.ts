import type { BaseClient } from "@evex/linejs/base";
import { TCompactProtocol } from "thrift";
import { LINEStruct } from "@evex/linejs/thrift";
import { setTimeout as delay } from "node:timers/promises";
import { PROTOCOL_VERSION, type NativeCore, type ReceivedBatch } from "../protocol/native.js";
import type { CoreEvent } from "../protocol/generated/CoreEvent.js";
import { ApiScheduler, errorCode } from "./api.js";
import { normalizeEvents } from "./events.js";
import type { SquareDirectory } from "./square.js";

type AccountPage = Awaited<ReturnType<BaseClient["square"]["fetchMyEvents"]>>;
type ChatPage = Awaited<ReturnType<BaseClient["square"]["fetchSquareChatEvents"]>>;
type SquareEvent = AccountPage["events"][number];
type ChatRetry = { attempts: number; retryAtMs: number };
type Checkpoint = { syncToken?: string; continuationToken?: string; originMs: number; subscriptionId?: number;
  pendingChats?: string[]; chatRetries?: Record<string, ChatRetry> };

export class Receiver {
  readonly metrics = { sessions: 0, signOns: 0, pushHints: 0, leaseRenewals: 0, pages: 0, events: 0, accepted: 0, duplicates: 0, ignored: 0, maxLagMs: 0,
    pendingChats: 0, chatFailures: 0, types: {} as Record<string, number> };
  status = "starting";
  private outgoingBytes = 0;

  constructor(private client: BaseClient, private core: NativeCore, private gate: ApiScheduler, private signal: AbortSignal, private directory?: SquareDirectory) {
    const nativePush = client.fetchPush;
    const transport = client as unknown as { fetchPush: typeof client.fetchPush };
    transport.fetchPush = (input, init) => {
      const request = new Request(input, init);
      if (!request.body) return nativePush(request);
      const reader = request.body.getReader();
      const body = new ReadableStream<Uint8Array>({
        pull: async controller => {
          const chunk = await reader.read();
          if (chunk.done) controller.close();
          else { this.outgoingBytes = Math.max(0, this.outgoingBytes - chunk.value.byteLength); controller.enqueue(chunk.value); }
        },
        cancel: reason => reader.cancel(reason),
      });
      return nativePush(new Request(request, { body, duplex: "half" } as RequestInit));
    };
  }

  async run(): Promise<void> {
    let backoff = 1000;
    while (!this.signal.aborted) {
      try { await this.session(); backoff = 1000; }
      catch (error) {
        if (this.signal.aborted) break;
        this.status = "reconnecting";
        console.log(JSON.stringify({ kind: "receiver-error", code: errorCode(error), backoffMs: backoff }));
        if ((error as Error).name === "CoreStoreError") throw error;
      }
      if (!this.signal.aborted) await delay(backoff + Math.floor(Math.random() * 300), undefined, { signal: this.signal });
      backoff = Math.min(60000, backoff * 2);
    }
    this.status = "stopped";
  }

  private readCheckpoint(stream: string, originMs: number): Checkpoint {
    try {
      const stored = this.core.checkpoint(stream);
      if (!stored) return { originMs };
      const checkpoint = JSON.parse(stored) as Checkpoint;
      if (!Number.isSafeInteger(checkpoint.originMs) || checkpoint.originMs <= 0
          || (checkpoint.subscriptionId !== undefined && (!Number.isSafeInteger(checkpoint.subscriptionId) || checkpoint.subscriptionId <= 0))
          || (checkpoint.pendingChats && (!Array.isArray(checkpoint.pendingChats) || checkpoint.pendingChats.length > 256
            || checkpoint.pendingChats.some(chat => typeof chat !== "string" || !chat || chat.length > 256)))
          || (checkpoint.chatRetries && (Object.keys(checkpoint.chatRetries).length > 256
            || Object.entries(checkpoint.chatRetries).some(([chat, retry]) => !checkpoint.pendingChats?.includes(chat) || !retry
              || !Number.isSafeInteger(retry.attempts) || retry.attempts < 1 || retry.attempts > 10
              || !Number.isSafeInteger(retry.retryAtMs) || retry.retryAtMs <= 0)))) throw new Error("InvalidCheckpoint");
      return checkpoint;
    } catch (error) { throw Object.assign(new Error(errorCode(error)), { name: "CoreStoreError" }); }
  }

  private async accept(stream: string, checkpoint: Checkpoint, events: SquareEvent[]): Promise<void> {
    if (!Array.isArray(events) || events.length > 100) throw new Error("InvalidPage");
    const normalized: CoreEvent[] = [];
    for (const event of events) {
      this.metrics.events++;
      const kind = String(event.type);
      if (kind in this.metrics.types || Object.keys(this.metrics.types).length < 64) this.metrics.types[kind] = (this.metrics.types[kind] ?? 0) + 1;
      const converted = await normalizeEvents(event, this.directory, checkpoint.originMs);
      if (!converted.length) { this.metrics.ignored++; continue; }
      for (const item of converted) this.metrics.maxLagMs = Math.max(this.metrics.maxLagMs, Date.now() - item.createdAtMs);
      normalized.push(...converted);
    }
    const chunks: CoreEvent[][] = [[]];
    let bytes = 0;
    for (const event of normalized) {
      const size = Buffer.byteLength(JSON.stringify(event));
      if (size > 256 * 1024) throw Object.assign(new Error("EventByteLimit"), { name: "CoreStoreError" });
      if (chunks.at(-1)!.length === 100 || bytes + size > 256 * 1024) { chunks.push([]); bytes = 0; }
      chunks.at(-1)!.push(event); bytes += size;
    }
    const previous = this.core.checkpoint(stream) ?? JSON.stringify({ originMs: checkpoint.originMs });
    try {
      for (let index = 0; index < chunks.length; index++) {
      const batch: ReceivedBatch = { protocolVersion: PROTOCOL_VERSION, streamKey: stream,
        checkpoint: index === chunks.length - 1 ? JSON.stringify(checkpoint) : previous,
        baselineBeforeMs: checkpoint.originMs, events: chunks[index] };
      const receipt = await this.core.submitBatchAsync(batch);
      this.metrics.accepted += receipt.accepted;
      this.metrics.duplicates += receipt.duplicates;
      if (receipt.actionsCreated) console.log(JSON.stringify({ kind: "accepted", source: stream === "account" ? "notification" : "chat",
        accepted: receipt.accepted, duplicates: receipt.duplicates, actions: receipt.actionsCreated }));
      }
    } catch (error) {
      // 容量超過・保存失敗時はcursorを進めず停止して原因を観測する。
      throw Object.assign(new Error(errorCode(error)), { name: "CoreStoreError" });
    }
    this.metrics.pages++;
  }

  private async drainChat(chat: string, originMs: number): Promise<boolean> {
    const stream = `chat:${chat}`;
    let checkpoint = this.readCheckpoint(stream, originMs);
    for (let pages = 0; pages < 4; pages++) {
      this.signal.throwIfAborted();
      // SDK公開wrapperで型に出ていないcontinuationTokenも生成済みThrift型を使って渡す。
      const response: ChatPage = await this.client.request.request(
        LINEStruct.SquareService_fetchSquareChatEvents_args({ request: {
          squareChatMid: chat, syncToken: checkpoint.syncToken, continuationToken: checkpoint.continuationToken,
          subscriptionId: checkpoint.subscriptionId, direction: "FORWARD", limit: 100, fetchType: "DEFAULT",
        } }), "fetchSquareChatEvents", this.client.square.protocolType, true, this.client.square.requestPath);
      if (typeof response.syncToken !== "string") throw new Error("InvalidChatCheckpoint");
      const subscriptionId = Number(response.subscription.subscriptionId);
      if (!Number.isSafeInteger(subscriptionId) || subscriptionId <= 0) throw new Error("InvalidSubscription");
      checkpoint = { originMs, syncToken: response.syncToken, continuationToken: response.continuationToken || undefined,
        subscriptionId };
      await this.accept(stream, checkpoint, response.events);
      if (!checkpoint.continuationToken) return true;
    }
    // 大量の補完でも、保存したcontinuationを残して他トーク・新着取得へ譲る。
    return false;
  }

  private async completePending(checkpoint: Checkpoint): Promise<Checkpoint> {
    const pending = [...(checkpoint.pendingChats ?? [])];
    const retries = { ...(checkpoint.chatRetries ?? {}) };
    const chats = pending.filter(chat => (retries[chat]?.retryAtMs ?? 0) <= Date.now()).slice(0, 2);
    if (!chats.length) return checkpoint;
    // 一度に2トークだけ処理する。失敗したトークの位置は残し、他の受信を止めない。
    const results = await Promise.allSettled(chats.map(chat => this.drainChat(chat, checkpoint.originMs)));
    this.signal.throwIfAborted();
    for (const [index, result] of results.entries()) {
      const chat = chats[index];
      if (result.status === "fulfilled") {
        pending.splice(pending.indexOf(chat), 1);
        delete retries[chat];
        if (!result.value) pending.push(chat);
      } else {
        if ((result.reason as Error)?.name === "CoreStoreError") throw result.reason;
        const attempts = Math.min(10, (retries[chat]?.attempts ?? 0) + 1);
        retries[chat] = { attempts, retryAtMs: Date.now() + Math.min(900000, 1000 * 2 ** attempts) };
        this.metrics.chatFailures++;
        console.log(JSON.stringify({ kind: "chat-catch-up-error", code: errorCode(result.reason), attempts }));
      }
    }
    checkpoint = { ...checkpoint, pendingChats: pending, chatRetries: retries };
    await this.accept("account", checkpoint, []);
    this.metrics.pendingChats = pending.length;
    return checkpoint;
  }

  private async session(): Promise<void> {
    const push = this.client.push;
    let checkpoint = this.readCheckpoint("account", Date.now());
    checkpoint = await this.completePending(checkpoint);
    this.client.poll.sync.square = checkpoint.syncToken;
    let dirty = false;
    let closed = false;
    let wake: (() => void) | undefined;
    let lastFrameAt = Date.now();
    let noopTask: Promise<void> | undefined;
    let initialResolve!: (page: AccountPage) => void;
    let initialReject!: (error: unknown) => void;
    const initial = new Promise<AccountPage>((resolve, reject) => { initialResolve = resolve; initialReject = reject; });
    // 初期取得中の拒否も必ず監視する。
    void initial.catch(() => {});
    push.onSignOnResponse = (requestId, _finished, data) => {
      lastFrameAt = Date.now();
      try {
        if (push.signOnRequests[requestId]?.[0] !== 3 || data.byteLength > 1024 * 1024) throw new Error("UnexpectedSignOn");
        delete push.signOnRequests[requestId];
        const result = this.client.thrift.rename_data(this.client.thrift.readThrift(data, TCompactProtocol), true).data;
        if (result.e) {
          const error = Object.assign(new Error("SignOnRejected"), { code: errorCode({ data: result.e }) });
          this.gate.cooldownFor(error);
          throw error;
        }
        if (!result.success) throw new Error("SignOnRejected");
        initialResolve(result.success as AccountPage);
      } catch (error) { initialReject(error); }
    };
    push.onPushResponse = frame => {
      lastFrameAt = Date.now();
      if (frame.serviceType !== 3) return;
      this.metrics.pushHints++;
      dirty = true;
      wake?.();
    };
    push.onPingCallback = id => {
      lastFrameAt = Date.now();
      if (id % 3 !== 0 || noopTask || closed || this.signal.aborted) return;
      noopTask = this.client.talk.noop().then(() => {
        if (push.authToken !== this.client.authToken) void push.conns[0]?.close();
      }).catch(error => console.log(JSON.stringify({ kind: "keepalive-error", code: errorCode(error) })))
        .finally(() => { noopTask = undefined; });
    };
    await this.gate.pace();
    this.outgoingBytes = 0;
    const initialize = push.initializeConn.bind(push) as unknown as (state: number, services: number[]) => ReturnType<typeof push.initializeConn>;
    const conn = await initialize(1, [3]);
    const close = () => { closed = true; wake?.(); void conn.close(); };
    this.signal.addEventListener("abort", close, { once: true });
    const watchdog = setInterval(() => { if (Date.now() - lastFrameAt > 90000) close(); }, 30000);
    let read: Promise<void> | undefined;
    try {
      // SDKの固定500ms待ちに依存せず、初回だけ時間を制限してHTTP/2応答を待つ。
      const connectDeadline = Date.now() + 15000;
      while (!conn.resStream) {
        if (closed || Date.now() >= connectDeadline) throw new Error("PushConnectTimeout");
        await delay(50, undefined, { signal: this.signal });
      }
      const onData = conn.onDataReceived.bind(conn);
      conn.onDataReceived = data => {
        lastFrameAt = Date.now();
        const buffered = Object.values(conn.notFinPayloads).reduce((sum, value) => sum + value.byteLength, 0);
        if (data.byteLength + conn.cacheData.byteLength + buffered > 1024 * 1024) throw new Error("PushBufferLimit");
        onData(data);
      };
      const write = conn.writeByte.bind(conn);
      conn.writeByte = async data => {
        if (this.outgoingBytes + data.byteLength > 64 * 1024) { initialReject(new Error("PushWriteLimit")); close(); return; }
        this.outgoingBytes += data.byteLength;
        try { await write(data); }
        catch (error) { initialReject(error); close(); }
      };
      this.metrics.sessions++;
      this.metrics.signOns++;
      this.status = "syncing";
      read = push.InitAndRead([3]);
      void read.then(() => initialReject(new Error("PushClosed")), initialReject).finally(close);
      const initialTimer = setTimeout(() => initialReject(new Error("SignOnTimeout")), 15000);
      let page: AccountPage;
      try { page = await initial; } finally { clearTimeout(initialTimer); }
      // 継続中のsnapshotはsignup応答で上書きせず、保存済みのページ位置から再開する。
      if (checkpoint.continuationToken) page = await this.client.square.fetchMyEvents({ syncToken: checkpoint.syncToken,
        continuationToken: checkpoint.continuationToken, subscriptionId: Number(page.subscription.subscriptionId), limit: 100 });
      let pages = 0;
      while (!closed && !this.signal.aborted) {
        if (!Array.isArray(page.events) || page.events.length > 100 || typeof page.syncToken !== "string") throw new Error("InvalidAccountPage");
        const chats = [...new Set(page.events.filter(event => event.payload?.notificationMessage?.requiredToFetchChatEvents)
          .map(event => event.payload.notificationMessage.squareChatMid))];
        if (chats.length > 100 || chats.some(chat => typeof chat !== "string" || !chat)) throw new Error("InvalidChatHint");
        const pendingChats = [...new Set([...(checkpoint.pendingChats ?? []), ...chats])];
        if (pendingChats.length > 256) throw Object.assign(new Error("PendingChatCapacity"), { name: "CoreStoreError" });
        checkpoint = { ...checkpoint, syncToken: page.syncToken,
          continuationToken: page.continuationToken || undefined, pendingChats };
        this.metrics.pendingChats = pendingChats.length;
        await this.accept("account", checkpoint, page.events);
        this.client.poll.sync.square = checkpoint.syncToken;
        push.subscriptionId = Number(page.subscription.subscriptionId);
        if (!Number.isSafeInteger(push.subscriptionId) || push.subscriptionId <= 0) throw new Error("InvalidSubscription");
        const ttl = Number(page.subscription.ttlMillis);
        // 購読期限でだけ更新する。ttlが得られない応答は30分を仮の観測値にする。
        const renewAt = Date.now() + (Number.isFinite(ttl) && ttl > 0 ? Math.max(1000, Math.min(86400000, ttl * 0.8)) : 1800000);
        checkpoint = await this.completePending(checkpoint);
        if (checkpoint.continuationToken) {
          if (++pages >= 100) throw new Error("AccountPageBudget");
        } else {
          pages = 0;
          this.status = "receiving";
          while (!dirty && !closed && !this.signal.aborted && Date.now() < renewAt) {
            checkpoint = await this.completePending(checkpoint);
            if (dirty || closed || this.signal.aborted) break;
            const retryAt = Math.min(renewAt, ...(checkpoint.pendingChats ?? []).map(chat => checkpoint.chatRetries?.[chat]?.retryAtMs ?? Date.now()));
            let timer: NodeJS.Timeout | undefined;
            try {
              await new Promise<void>(resolve => { wake = resolve; timer = setTimeout(resolve, Math.max(0, retryAt - Date.now())); });
            } finally { if (timer) clearTimeout(timer); }
          }
          if (Date.now() >= renewAt) this.metrics.leaseRenewals++;
          wake = undefined;
        }
        if (closed || this.signal.aborted) break;
        // 継続ページ中に来た通知は全ページ終了後の再取得まで残す。
        if (!checkpoint.continuationToken) dirty = false;
        page = await this.client.square.fetchMyEvents({ syncToken: checkpoint.syncToken,
          continuationToken: checkpoint.continuationToken, subscriptionId: push.subscriptionId, limit: 100 });
      }
    } finally {
      clearInterval(watchdog);
      this.signal.removeEventListener("abort", close);
      close();
      await read?.catch(() => {});
      await noopTask;
      await conn.resStream?.cancel().catch(() => {});
      push.conns = [];
    }
  }
}
