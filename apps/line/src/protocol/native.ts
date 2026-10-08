import { createRequire } from "node:module";
import type { CoreConfig } from "./generated/CoreConfig.js";
import type { ReceivedBatch } from "./generated/ReceivedBatch.js";
import type { BatchReceipt } from "./generated/BatchReceipt.js";
import type { CoreAction } from "./generated/CoreAction.js";
import type { ActionResult } from "./generated/ActionResult.js";
import type { CoreStats } from "./generated/CoreStats.js";
import { PROTOCOL_VERSION } from "./generated/version.js";
import type { PendingLog } from "./generated/PendingLog.js";
import type { RuntimeStatus } from "./generated/RuntimeStatus.js";

export { PROTOCOL_VERSION };
export type { CoreConfig, ReceivedBatch, BatchReceipt, CoreAction, ActionResult, CoreStats, RuntimeStatus };

export interface NativeCore {
  updateRuntimeStatus(status: RuntimeStatus): void;
  checkpoint(stream: string): string | null;
  priorityChats(): string[];
  submitBatch(batch: ReceivedBatch): BatchReceipt;
  submitBatchAsync(batch: ReceivedBatch): Promise<BatchReceipt>;
  prepareImage(actionId: string): Promise<Buffer | null>;
  runMediaJobs(): Promise<void>;
  runStoreMonitors(): Promise<void>;
  prepareAttachment(actionId: string): Promise<Buffer | null>;
  nextAction(): Promise<CoreAction | null>;
  nextQueryAction(): Promise<CoreAction | null>;
  markSending(actionId: string): void;
  retryAction(actionId: string, delayMs: number): void;
  completeAction(result: ActionResult): void;
  resolveAction(result: ActionResult): void;
  stats(): CoreStats;
  shutdown(): void;
  persistenceRevision(): string;
  snapshotDatabase(path: string): Promise<void>;
  pendingLogs(): PendingLog[];
  acknowledgeLogs(sequences: number[]): void;
}

export function createCore(config: CoreConfig, runtimeStatus?: () => RuntimeStatus): NativeCore {
  const native = createRequire(import.meta.url)("../../native/kbc_node.node");
  if (native.getRuntimeInfo().protocolVersion !== PROTOCOL_VERSION) throw new Error("ProtocolMismatch");
  // JSON文字列で整数表現を保ち、N-APIのValue変換で時刻がf64になる問題を避ける。
  const handle = native.createCore(JSON.stringify(config));
  const update = () => { if (runtimeStatus) handle.updateRuntimeStatus(JSON.stringify(runtimeStatus())); };
  return {
    updateRuntimeStatus: status => handle.updateRuntimeStatus(JSON.stringify(status)),
    checkpoint: stream => handle.checkpoint(stream),
    priorityChats: () => handle.priorityChats(),
    submitBatch: batch => { update(); return handle.submitBatch(JSON.stringify(batch)); },
    submitBatchAsync: batch => { update(); return handle.submitBatchAsync(JSON.stringify(batch)); },
    prepareImage: actionId => handle.prepareImage(actionId),
    runMediaJobs: () => handle.runMediaJobs(),
    runStoreMonitors: () => handle.runStoreMonitors(),
    prepareAttachment: actionId => handle.prepareAttachment(actionId),
    nextAction: () => handle.nextAction(),
    nextQueryAction: () => handle.nextQueryAction(),
    markSending: actionId => handle.markSending(actionId),
    retryAction: (actionId, delayMs) => handle.retryAction(actionId, delayMs),
    completeAction: result => handle.completeAction(JSON.stringify(result)),
    resolveAction: result => handle.resolveAction(JSON.stringify(result)),
    stats: () => handle.stats(),
    shutdown: () => handle.shutdown(),
    persistenceRevision: () => handle.persistenceRevision(),
    snapshotDatabase: path => handle.snapshotDatabase(path),
    pendingLogs: () => handle.pendingLogs(),
    acknowledgeLogs: sequences => handle.acknowledgeLogs(JSON.stringify(sequences)),
  };
}
