import type { BaseClient } from "@evex/linejs/base";
import type { CoreAction, NativeCore } from "../protocol/native.js";
import { ApiScheduler, errorCode, type SendAttempt } from "./api.js";

export async function deliverAction(client: BaseClient, core: NativeCore, gate: ApiScheduler, action: CoreAction): Promise<{ status: "sent" | "unknown" | "queued"; code: string }> {
  const message = action.type === "sendMessage" ? action : null;
  const bytes = message?.imageUrl ? await core.prepareImage(action.actionId) : null;
  if (message?.imageUrl && !bytes) return { status: "queued", code: "ImageUnavailable" };
  let storeError: unknown;
  const attempt: SendAttempt = { started: false, method: message ? "sendMessage" : "destroyMessage", beforeSend: () => {
    try { core.markSending(action.actionId); }
    catch (error) { storeError = error; throw error; }
  } };
  let code = "OK";
  let messageId: string | undefined;
  try {
    if (action.type === "sendMessage") {
      const sent = await gate.withSendAttempt(attempt, () => client.square.sendMessage({ squareChatMid: action.chatId,
        relatedMessageId: action.relatedMessageId, text: action.imageUrl ? undefined : action.text,
        contentType: action.imageUrl ? "IMAGE" : undefined }));
      messageId = sent.createdSquareMessage?.message?.id;
      if (!messageId) throw new Error("MissingSentMessageId");
      if (bytes && action.imageUrl) {
        const blob = new Blob([Uint8Array.from(bytes)], { type: "image/png" });
        await gate.run("uploadImage", () => client.obs.uploadObjTalk(action.chatId, "image", blob, messageId,
          new URL(action.imageUrl!).pathname.split("/").at(-1)));
      }
    } else {
      await gate.withSendAttempt(attempt, () => client.square.destroyMessage({ squareChatMid: action.chatId, messageId: action.messageId }));
    }
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
  core.completeAction({ actionId: action.actionId, status: "sent", code, messageId });
  return { status: "sent", code };
}
