import { createRequire } from "node:module";
import type { CoreConfig } from "./generated/CoreConfig.js";
import type { ReceivedBatch } from "./generated/ReceivedBatch.js";
import type { BatchReceipt } from "./generated/BatchReceipt.js";
import type { CoreAction } from "./generated/CoreAction.js";
import type { ActionResult } from "./generated/ActionResult.js";
import type { CoreStats } from "./generated/CoreStats.js";
import { PROTOCOL_VERSION } from "./generated/version.js";

export { PROTOCOL_VERSION };
export type { CoreConfig, ReceivedBatch, BatchReceipt, CoreAction, ActionResult, CoreStats };

export interface NativeCore {
  checkpoint(stream: string): string | null;
  submitBatch(batch: ReceivedBatch): BatchReceipt;
  nextAction(): Promise<CoreAction | null>;
  markSending(actionId: string): void;
  retryAction(actionId: string, delayMs: number): void;
  completeAction(result: ActionResult): void;
  resolveAction(result: ActionResult): void;
  stats(): CoreStats;
  shutdown(): void;
}

export function createCore(config: CoreConfig): NativeCore {
  const native = createRequire(import.meta.url)("../../native/kbc_node.node");
  if (native.getRuntimeInfo().protocolVersion !== PROTOCOL_VERSION) throw new Error("ProtocolMismatch");
  // JSON文字列で整数表現を保ち、N-APIのValue変換で時刻がf64になる問題を避ける。
  const handle = native.createCore(JSON.stringify(config));
  return {
    checkpoint: stream => handle.checkpoint(stream),
    submitBatch: batch => handle.submitBatch(JSON.stringify(batch)),
    nextAction: () => handle.nextAction(),
    markSending: actionId => handle.markSending(actionId),
    retryAction: (actionId, delayMs) => handle.retryAction(actionId, delayMs),
    completeAction: result => handle.completeAction(JSON.stringify(result)),
    resolveAction: result => handle.resolveAction(JSON.stringify(result)),
    stats: () => handle.stats(),
    shutdown: () => handle.shutdown(),
  };
}
