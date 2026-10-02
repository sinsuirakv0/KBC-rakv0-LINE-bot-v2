import type { BaseClient } from "@evex/linejs/base";
import type { CoreAction, NativeCore } from "../protocol/native.js";
import { ApiScheduler, errorCode, type SendAttempt } from "./api.js";

export async function deliverAction(client: BaseClient, core: NativeCore, gate: ApiScheduler, action: CoreAction): Promise<{ status: "sent" | "unknown" | "queued"; code: string }> {
  let storeError: unknown;
  const attempt: SendAttempt = { started: false, beforeSend: () => {
    try { core.markSending(action.actionId); }
    catch (error) { storeError = error; throw error; }
  } };
  let code = "OK";
  try {
    await gate.withSendAttempt(attempt, () => client.square.sendMessage({ squareChatMid: action.chatId,
      relatedMessageId: action.relatedMessageId, text: action.text }));
    if (!attempt.started) throw new Error("SendBoundaryNotReached");
  } catch (error) {
    code = errorCode(error);
    if (!attempt.started) {
      core.retryAction(action.actionId, 1000);
      if (storeError) throw storeError;
      if (code === "SendBoundaryNotReached") throw error;
      return { status: "queued", code };
    }
    core.completeAction({ actionId: action.actionId, status: "unknown", code });
    return { status: "unknown", code };
  }
  core.completeAction({ actionId: action.actionId, status: "sent", code });
  return { status: "sent", code };
}
