import type { BaseClient } from "@evex/linejs/base";
import type { CoreAction, NativeCore } from "../protocol/native.js";
import { ApiScheduler, errorCode, type SendAttempt } from "./api.js";
import { SquareDirectory, mentionMetadata } from "./square.js";

export async function deliverAction(client: BaseClient, core: NativeCore, gate: ApiScheduler, action: CoreAction, directory?: SquareDirectory): Promise<{ status: "sent" | "failed" | "unknown" | "queued"; code: string }> {
  if (action.type === "prepareMedia") throw new Error("InternalActionReachedAdapter");
  if (action.type === "ocApi") {
    const service = directory ?? new SquareDirectory(client);
    const read = ["context", "member", "chats", "members", "joinedChats", "inspect"].includes(action.request.type);
    if (read) {
      core.markSending(action.actionId);
      let result;
      try { result = await service.execute(action.chatId, action.request); }
      catch (error) {
        const code = errorCode(error);
        core.completeAction({ actionId: action.actionId, status: "failed", code });
        return { status: "failed", code };
      }
      core.completeAction({ actionId: action.actionId, status: "sent", code: "OK", ocResult: result });
      return { status: "sent", code: "OK" };
    }
    let storeError: unknown;
    const method: SendAttempt["method"] = action.request.type === "report" ? "reportSquareMessage" : action.request.type === "roles" ? "updateSquareMembers"
      : action.request.type === "post" ? "sendMessage" : action.request.type === "delete" ? "destroyMessage" : "updateSquareMember";
    const attempt: SendAttempt = { started: false, method, beforeSend: () => {
      if (Date.now() - action.createdAtMs > 30_000) throw new Error("OcActionExpired");
      try { core.markSending(action.actionId); } catch (error) { storeError = error; throw error; }
    } };
    let result;
    try { result = await gate.withSendAttempt(attempt, () => service.execute(action.chatId, action.request)); }
    catch (error) {
      if (storeError) throw storeError;
      const code = errorCode(error);
      if (!attempt.started) {
        if (code === "OcActionExpired") { core.completeAction({ actionId: action.actionId, status: "failed", code }); return { status: "failed", code }; }
        core.retryAction(action.actionId, 1000); return { status: "queued", code };
      }
      core.completeAction({ actionId: action.actionId, status: "unknown", code }); return { status: "unknown", code };
    }
    if (!attempt.started) throw new Error("SendBoundaryNotReached");
    core.completeAction({ actionId: action.actionId, status: "sent", code: "OK", ocResult: result });
    return { status: "sent", code: "OK" };
  }
  const message = action.type === "sendMessage" ? action : null;
  const bytes = message?.attachment ? await core.prepareAttachment(action.actionId) : message?.imageUrl ? await core.prepareImage(action.actionId) : null;
  if (message?.attachment && !bytes) return { status: "queued", code: "MediaUnavailable" };
  if (message?.imageUrl && !bytes) return { status: "queued", code: "ImageUnavailable" };
  let storeError: unknown;
  const attempt: SendAttempt = { started: false, method: message ? bytes ? "uploadMedia" : "sendMessage" : "destroyMessage", beforeSend: () => {
    try { core.markSending(action.actionId); }
    catch (error) { storeError = error; throw error; }
  } };
  let code = "OK";
  let messageId: string | undefined;
  try {
    if (action.type === "sendMessage") {
      if (bytes) {
        // OCはoid省略のOBS upload自身が投稿する。空のIMAGE/VIDEOを先に送らない。
        const media = action.attachment;
        const blob = new Blob([Uint8Array.from(bytes)], { type: media?.contentType ?? "image/png" });
        const kind = (media?.kind ?? "image") as "image" | "gif" | "video" | "file";
        const uploaded = await gate.withSendAttempt(attempt, () => gate.run("uploadMedia", () => client.obs.uploadObjTalk(action.chatId, kind, blob, undefined,
          media?.fileName ?? new URL(action.imageUrl!).pathname.split("/").at(-1), media?.durationMs)));
        messageId = uploaded.objId;
        if (!messageId) throw new Error("MissingSentMessageId");
      } else {
        const sent = await gate.withSendAttempt(attempt, () => client.square.sendMessage({ squareChatMid: action.chatId,
          relatedMessageId: action.relatedMessageId || undefined, text: action.text,
          contentMetadata: mentionMetadata(action.mention) }));
        messageId = sent.createdSquareMessage?.message?.id;
        if (!messageId) throw new Error("MissingSentMessageId");
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
