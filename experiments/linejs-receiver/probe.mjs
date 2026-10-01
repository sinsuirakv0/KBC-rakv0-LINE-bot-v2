import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { createServer as createHttpServer } from "node:http";
import { createServer as createTcpServer } from "node:net";
import { setTimeout as delay, setImmediate as nextTurn } from "node:timers/promises";
import { performance } from "node:perf_hooks";

const packagePath = "./node_modules/@evex/linejs/";
const sdk = JSON.parse(await readFile(new URL(packagePath + "package.json", import.meta.url)));
const lock = JSON.parse(await readFile(new URL("./package-lock.json", import.meta.url)));
assert.equal(sdk.version, "3.4.2");
const observations = [];
const dispatchers = new Set();
const nativeFetch = globalThis.fetch;
let localRequests = 0;

// 検証中の実通信はloopbackだけ許可する。認証・実LINE接続は行わない。
globalThis.fetch = (input, init) => {
  const url = new URL(input instanceof Request ? input.url : input);
  assert.equal(url.hostname, "127.0.0.1", "external network is forbidden");
  localRequests++;
  if (init?.dispatcher) dispatchers.add(init.dispatcher);
  return nativeFetch(input, init);
};

const { ConnManager } = await import(packagePath + "base/push/connManager.js");
const { Conn } = await import(packagePath + "base/push/conn.js");
const { Polling } = await import(packagePath + "base/polling/mod.js");
const { Client } = await import(packagePath + "client/client.js");
const { BaseClient } = await import(packagePath + "base/core/mod.js");
const { LegyEncryptedTransport } = await import(packagePath + "base/request/legy.js");
const { createNodeFetch } = await import(packagePath + "base/core/node_fetch.js");

function eventAt(index) {
  return {
    type: "NOTIFICATION_MESSAGE",
    payload: { notificationMessage: { squareMessage: { message: {
      id: String(index), to: "offline-chat-" + index % 3,
      from: "offline-member", text: "o.ping", createdTime: "0",
    } } } },
  };
}
function messageId(event) {
  return event.payload.notificationMessage.squareMessage.message.id;
}
function makeWriter() {
  return ConnManager.prototype.createAsyncReadableStream.call({});
}
function record(name, details) {
  observations.push({ name, ...details });
}
async function readEvents(stream, count) {
  const reader = stream.getReader();
  const events = [];
  try {
    for (let index = 0; index < count; index++) {
      const item = await reader.read();
      assert.equal(item.done, false);
      events.push(item.value);
    }
  } finally {
    reader.releaseLock();
  }
  return events;
}
async function streamBoundaries() {
  const count = 500;
  const writer = makeWriter();
  for (let index = 0; index < count; index++) writer.enqueue(eventAt(index));
  const events = await readEvents(writer.stream, count);
  assert.deepEqual(events.map(messageId), Array.from({ length: count }, (_, index) => String(index)));
  record("burst", { sent: count, delivered: events.length, chatCount: 3, ordered: true, sameTextAndTime: true });

  for (const operation of ["renew", "cancel", "error"]) {
    const pending = makeWriter();
    for (let index = 0; index < count; index++) pending.enqueue(eventAt(index));
    const oldStream = pending.stream;
    if (operation === "cancel") await oldStream.cancel();
    else if (operation === "error") pending.error(new Error("offline-stream-failure"));
    else pending.renew();
    const recovered = await readEvents(pending.stream, 300);
    assert.equal(messageId(recovered[0]), "200");
    const retained = operation === "renew" ? await readEvents(oldStream, 200) : [];
    assert.equal(retained.length, operation === "renew" ? 200 : 0);
    record("stream-" + operation, {
      queuedBefore: count, newStreamDelivered: recovered.length,
      firstNewId: messageId(recovered[0]), readableFromOldStream: retained.length,
      internalQueueNotTransferred: 200,
    });
  }
}
async function pagesAndCursor() {
  const events = Array.from({ length: 250 }, (_, index) => eventAt(index));
  const calls = [];
  const emitted = [];
  const client = {
    poll: { sync: { square: "checkpoint-0" } },
    thrift: { readThriftStruct: () => ({ 1: 1 }) },
    square: { fetchMyEvents: async (input) => {
      calls.push({ ...input });
      return { events: events.slice(0, 100), syncToken: "checkpoint-1",
        continuationToken: "page-2", subscription: { subscriptionId: 1 } };
    } },
    emit: (type) => emitted.push(type), log() {},
  };
  const manager = new ConnManager(client);
  await manager._OnPushResponse({ serviceType: 3, pushPayload: new Uint8Array([1]) });
  const cursorBeforeConsumer = client.poll.sync.square;
  assert.equal(cursorBeforeConsumer, "checkpoint-1");
  const received = await readEvents(manager.sqStream.stream, 100);
  assert.equal(calls.length, 1);
  assert.equal(client.poll.sync.square, "checkpoint-1");
  record("push-page", { availableInMock: 250, fetched: received.length, calls: calls.length,
    continuationDrained: false, cursorBeforeConsumer, cursorChangedBeforeConsumer: true, emitted: [...emitted] });

  const abort = new AbortController();
  const pageCalls = [];
  const polling = new Polling({ square: { fetchMyEvents: async (input) => {
    pageCalls.push({ ...input });
    const offset = input.continuationToken ? Number(input.continuationToken) : 0;
    return { events: events.slice(offset, offset + 100), syncToken: "checkpoint-" + pageCalls.length,
      continuationToken: offset + 100 < events.length ? String(offset + 100) : undefined };
  } } });
  polling.sync.square = "checkpoint-0";
  const ids = [];
  for await (const event of polling._listenSquareEvents({ signal: abort.signal, pollingInterval: 0 })) {
    ids.push(messageId(event));
    if (ids.length === events.length) abort.abort();
  }
  assert.equal(ids.length, 250);
  assert.deepEqual(pageCalls.map((call) => call.continuationToken ?? null), [null, "100", "200"]);
  record("raw-pages", { delivered: ids.length, calls: pageCalls.length, continuationTokens: [null, "100", "200"] });

  // 後の要求が先に完了する模擬条件で、SDK側の直列化とcursor更新を確認する。
  const replies = [Promise.withResolvers(), Promise.withResolvers()];
  const concurrentCalls = [];
  client.poll.sync.square = "checkpoint-0";
  client.square.fetchMyEvents = (input) => {
    concurrentCalls.push({ ...input });
    return replies[concurrentCalls.length - 1].promise;
  };
  const first = manager._OnPushResponse({ serviceType: 3, pushPayload: new Uint8Array([1]) });
  const second = manager._OnPushResponse({ serviceType: 3, pushPayload: new Uint8Array([1]) });
  replies[1].resolve({ events: [], syncToken: "newer", subscription: { subscriptionId: 1 } });
  await second;
  replies[0].resolve({ events: [], syncToken: "older", subscription: { subscriptionId: 1 } });
  await first;
  assert.equal(client.poll.sync.square, "older");
  record("concurrent-push", { concurrentFetches: concurrentCalls.length,
    requestedCursors: concurrentCalls.map((call) => call.syncToken), finalCursor: client.poll.sync.square,
    responseOrder: [2, 1], serverBehavior: "mock-only" });
}
async function clientDelivery() {
  for (const mode of ["normal", "sync-error", "async-handler"]) {
    const writer = makeWriter();
    const errors = [];
    const base = { createPolling: () => ({ listenSquareEvents: () => writer.stream }),
      log: (_, value) => errors.push(value.error.message) };
    const client = new Client(base);
    const raw = [];
    const messages = [];
    const completions = [];
    const gate = Promise.withResolvers();
    client.on("square:event", mode === "async-handler" ? async (event) => {
      const id = messageId(event);
      raw.push(id);
      if (id === "0") await gate.promise;
      completions.push(id);
    } : (event) => {
      raw.push(messageId(event));
      if (mode === "sync-error" && messageId(event) === "0") throw new Error("offline-listener-failure");
    });
    client.on("square:message", (message) => messages.push(message.raw.message.id));
    client.listen({ talk: false, square: true });
    writer.enqueue(eventAt(0));
    writer.enqueue(eventAt(1));
    writer.close();
    await nextTurn();
    if (mode === "async-handler") { gate.resolve(); await nextTurn(); }
    assert.deepEqual(raw, ["0", "1"]);
    assert.deepEqual(messages, mode === "sync-error" ? ["1"] : ["0", "1"]);
    if (mode === "async-handler") assert.deepEqual(completions, ["1", "0"]);
    record("client-" + mode, { rawIds: raw, messageIds: messages, errors, asyncCompletionOrder: completions });
  }
}
async function startupAndPushError() {
  const failure = new Error("offline-startup-failure");
  let attempts = 0;
  const client = { authToken: "offline-stub", log() {} };
  const manager = new ConnManager(client);
  client.push = manager;
  manager.initializeConn = async () => { attempts++; if (attempts === 1) throw failure; client.authToken = undefined; };
  manager.InitAndRead = async () => { manager.sqStream.enqueue(eventAt(0)); manager.sqStream.close(); };
  const polling = new Polling(client);
  const talkReader = manager.opStream.stream.getReader();
  const talkFailure = talkReader.read().catch((error) => error);
  const squareReader = polling.listenSquareEvents().getReader();
  const squareFailure = squareReader.read().catch((error) => error);
  assert.equal(await squareFailure, failure);
  assert.equal(await talkFailure, failure);
  assert.equal(polling.islisten, false);
  talkReader.releaseLock();
  squareReader.releaseLock();
  const retryStream = polling.listenSquareEvents();
  const retried = await readEvents(retryStream, 1);
  await delay(4100);
  assert.equal(polling.islisten, false);
  record("startup-retry", { attempts, bothReadersReceivedOriginalError: true,
    pollingStateReset: true, retryDelivered: retried.length });

  // SDKのPUSH callbackが返すrejectを、検証プロセス内だけで観測する。
  const pushFailure = new Error("offline-api-failure");
  const rejected = Promise.withResolvers();
  const observer = (error) => rejected.resolve(error);
  process.once("unhandledRejection", observer);
  try {
    const failing = new ConnManager({ log() {}, thrift: { readThriftStruct: () => ({ 1: 1 }) },
      poll: { sync: { square: "checkpoint-0" } }, square: { fetchMyEvents: async () => { throw pushFailure; } } });
    new Conn(failing).onPacketReceived(4, new Uint8Array([0, 3, 0, 0, 0, 1, 0]));
    const error = await Promise.race([rejected.promise, delay(500).then(() => null)]);
    assert.equal(error, pushFailure);
    record("push-callback-error", { unhandledRejectionObserved: true, apiCalls: "mock-only" });
  } finally {
    process.off("unhandledRejection", observer);
  }
}

async function pushReadiness() {
  let aborted = false;
  const client = {
    log() {},
    fetchPush: async (_, init) => new Promise((resolve, reject) => {
      const timer = setTimeout(() => resolve(new Response(new ReadableStream({
        start(controller) { controller.close(); },
      }))), 1200);
      init.signal.addEventListener("abort", () => {
        clearTimeout(timer);
        aborted = true;
        reject(init.signal.reason);
      }, { once: true });
    }),
  };
  const connection = new Conn(new ConnManager(client));
  const started = performance.now();
  await connection.new("offline.invalid", 443, "/PUSH", {});
  const error = await connection.read().catch((error) => error);
  await connection.close();
  assert.equal(error.message, "no resStream");
  assert.equal(aborted, true);
  record("push-delayed-readiness", {
    responseReadyAfterMs: 1200, failedAfterMs: Math.round(performance.now() - started),
    error: error.message, cancellationReachedMockTransport: aborted, transport: "mock",
  });
}

async function cancellationAndTransport() {
  const abort = new AbortController();
  let outerAborted = false;
  const transport = new LegyEncryptedTransport("https://offline.invalid/enc");
  const request = new Request("https://offline.invalid/SQ1", { method: "POST", body: new Uint8Array([1]), signal: abort.signal });
  const result = transport.fetch(request, async (outer) => {
    abort.abort(new DOMException("offline-cancel", "AbortError"));
    outerAborted = outer.signal.aborted;
    throw outer.signal.reason;
  }, { application: "OFFLINE", userAgent: "offline-probe" }).catch((error) => error);
  assert.equal((await result).name, "AbortError");
  assert.equal(outerAborted, true);
  record("legy-cancel", { outerSignalAborted: outerAborted, outcome: "AbortError", transport: "mock" });

  const routes = [];
  const base = new BaseClient({ device: "DESKTOPWIN", fetch: async (input) => {
    routes.push(new URL(input.url).pathname); return new Response(null, { status: 200 });
  } });
  await base.fetch("https://offline.invalid/RPC");
  await base.fetchPush("https://offline.invalid/PUSH");
  assert.deepEqual(routes, ["/RPC", "/PUSH"]);
  record("custom-fetch", { routes, pushUsesSameCustomTransport: true });

  const httpSockets = new Set();
  let requestAborted = false;
  const requestSeen = Promise.withResolvers();
  const httpServer = createHttpServer((incoming) => {
    incoming.on("aborted", () => { requestAborted = true; });
    requestSeen.resolve();
  });
  httpServer.on("connection", (socket) => { httpSockets.add(socket); socket.on("close", () => httpSockets.delete(socket)); });
  await new Promise((resolve) => httpServer.listen(0, "127.0.0.1", resolve));
  try {
    const rpcFetch = await createNodeFetch(false)(200);
    const rpcAbort = new AbortController();
    const pending = rpcFetch("http://127.0.0.1:" + httpServer.address().port + "/", { signal: rpcAbort.signal }).catch((error) => error);
    await requestSeen.promise;
    rpcAbort.abort();
    const error = await pending;
    await delay(50);
    assert.equal(requestAborted, true);
    record("node-rpc-abort", { errorName: error.name, serverObservedAbort: requestAborted, network: "loopback" });
  } finally {
    for (const socket of httpSockets) socket.destroy();
    await new Promise((resolve) => httpServer.close(resolve));
  }

  const tcpSockets = new Set();
  const tcpServer = createTcpServer((socket) => {
    tcpSockets.add(socket);
    socket.on("close", () => tcpSockets.delete(socket));
    socket.on("error", () => {});
    socket.resume();
  });
  await new Promise((resolve) => tcpServer.listen(0, "127.0.0.1", resolve));
  try {
    for (const allowH2 of [false, true]) {
      const fetch = await createNodeFetch(allowH2)(100);
      const started = performance.now();
      const error = await fetch("https://127.0.0.1:" + tcpServer.address().port + "/", {
        signal: AbortSignal.timeout(2000),
      }).catch((error) => error);
      const elapsedMs = Math.round(performance.now() - started);
      await delay(50);
      assert.equal(error.cause?.code, "UND_ERR_CONNECT_TIMEOUT");
      record(allowH2 ? "node-push-connect-timeout" : "node-rpc-connect-timeout", {
        configuredMs: 100, elapsedMs, errorName: error.name, causeCode: error.cause.code,
        socketsOpenBeforeCleanup: tcpSockets.size, network: "loopback-tls-handshake",
      });
    }
  } finally {
    for (const socket of tcpSockets) socket.destroy();
    await new Promise((resolve) => tcpServer.close(resolve));
  }
}

const watchdog = setTimeout(() => { throw new Error("probe exceeded 25 seconds"); }, 25000);
try {
  await streamBoundaries();
  await pagesAndCursor();
  await clientDelivery();
  await startupAndPushError();
  await pushReadiness();
  await cancellationAndTransport();
  const report = {
    recordedAt: new Date().toISOString(), sdkVersion: sdk.version, nodeVersion: process.version,
    undiciVersion: lock.packages["node_modules/undici"].version,
    sdkIntegrity: lock.packages["node_modules/@evex/linejs"].integrity,
    lineNetwork: false, localRequests, observations,
  };
  await writeFile(new URL("./results.json", import.meta.url), JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify(report, null, 2));
} finally {
  clearTimeout(watchdog);
  await Promise.all([...dispatchers].map((dispatcher) => dispatcher.destroy()));
}

