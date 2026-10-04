import type { CoreEvent } from "../protocol/generated/CoreEvent.js";
import type { BaseClient } from "@evex/linejs/base";
import type { SquareDirectory } from "./square.js";

type Event = Awaited<ReturnType<BaseClient["square"]["fetchMyEvents"]>>["events"][number];
type RecordValue = Record<string, unknown>;
const record = (value: unknown): RecordValue => value && typeof value === "object" ? value as RecordValue : {};
const string = (value: unknown): string => typeof value === "string" ? value : "";
const name = (value: unknown): string => Array.from(string(value)).slice(0, 80).join("").replace(/[\r\n]/g, " ");
const timestamp = (value: unknown): number | undefined => { const number = Number(value); return Number.isSafeInteger(number) && number > 0 ? number : undefined; };
const sequence = (value: unknown): number | undefined => { const number = timestamp(value); return number && number <= 1_000_000 ? number : undefined; };

export async function normalizeEvent(event: Event, directory?: SquareDirectory, baselineBeforeMs = 0, source = "push"): Promise<CoreEvent | null> {
  const payload = event.payload;
  if (["47", "NOTIFICATION_MESSAGE_REACTION"].includes(String(event.type))) {
    const reaction = payload?.notificationMessageReaction;
    const created = timestamp(event.createdTime);
    const reactionType = ({ "2": "NICE", NICE: "NICE", "3": "LOVE", LOVE: "LOVE" } as Record<string, string>)[String(reaction?.type)];
    if (!reaction?.squareChatMid || !reaction.messageId || !created || created < baselineBeforeMs || !reactionType) return null;
    // 通知の表示名で本人を判定せず、Coreが必要な一覧のreactor MIDを照会する。
    return { type: "reactionNotified", eventId: `reaction:${reaction.squareChatMid}:${reaction.messageId}:${reactionType}:${created}`,
      chatId: reaction.squareChatMid, messageId: reaction.messageId, reactionType, createdAtMs: created };
  }
  const message = (payload?.notificationMessage ?? payload?.receiveMessage)?.squareMessage?.message;
  if (message) {
    const created = timestamp(message.createdTime);
    if (!message.id || !message.to || !created) return null;
    let chat;
    try { chat = directory && created >= baselineBeforeMs ? await directory.chat(message.to) : undefined; }
    catch { /* OC情報の照会失敗でも本文を受付する。処分の所属・権限は未確認として扱う。 */ }
    const metadata = message.contentMetadata ?? {};
    let mentions: string[] = [];
    if (metadata.MENTION && metadata.MENTION.length <= 8192) {
      try { const parsed = JSON.parse(metadata.MENTION) as { MENTIONEES?: { M?: unknown }[] };
        if (Array.isArray(parsed.MENTIONEES)) mentions = [...new Set(parsed.MENTIONEES.map(item => string(item?.M)).filter(mid => /^p[0-9a-f]{8,63}$/i.test(mid)))].slice(0, 9);
      } catch { /* 壊れたメンションは処分対象の選択に使わない。 */ }
    }
    return { type: "messageReceived", eventId: `${message.to}:${message.id}`, chatId: message.to, messageId: message.id,
      text: message.text ?? "", senderId: message.from || undefined, createdAtMs: created,
      squareId: chat?.squareChat.squareMid ?? payload?.receiveMessage?.squareMid,
      botMemberId: chat?.squareChatMember.squareMemberMid,
      mentions, contentType: String(message.contentType ?? "TEXT"), mediaGroupId: metadata.GID || undefined,
      senderName: string(record(payload?.notificationMessage ?? payload?.receiveMessage).senderDisplayName) || undefined,
      metadataJson: JSON.stringify({ source, hasContent: message.hasContent, contentMetadata: metadata,
        relatedMessageId: message.relatedMessageId, relatedMessageServiceCode: message.relatedMessageServiceCode,
        messageRelationType: message.messageRelationType, location: message.location },
        (_key, value: unknown) => typeof value === "bigint" ? String(value) : value),
      mediaGroupSequence: sequence(metadata.GSEQ), mediaGroupTotal: sequence(metadata.GTOTAL),
      replyToMessageId: ["REPLY", "3"].includes(String(message.messageRelationType)) && message.relatedMessageId ? message.relatedMessageId : undefined };
  }
  const type = String(event.type), raw = record(payload);
  let member: RecordValue = {}, chatId = "", memberId = "", squareId = "", scope = "chat", state = "";
  if (["15", "NOTIFIED_CREATE_SQUARE_MEMBER"].includes(type)) { member = record(record(raw.notifiedCreateSquareMember).squareMember); scope = "square"; }
  else if (["11", "NOTIFIED_UPDATE_SQUARE_MEMBER"].includes(type)) {
    const update = record(raw.notifiedUpdateSquareMember); member = record(update.squareMember); scope = "square";
    squareId = string(update.squareMid); memberId = string(update.squareMemberMid);
  } else if (["12", "NOTIFIED_UPDATE_SQUARE_MEMBER_PROFILE"].includes(type)) {
    const update = record(raw.notifiedUpdateSquareMemberProfile); member = record(update.squareMember);
    chatId = string(update.squareChatMid); scope = "square"; state = "NAME";
  } else if (["16", "NOTIFIED_CREATE_SQUARE_CHAT_MEMBER"].includes(type)) {
    const update = record(raw.notifiedCreateSquareChatMember), chat = record(update.chat), chatMember = record(update.chatMember);
    member = record(update.peerSquareMember); chatId = string(chat.squareChatMid) || string(chatMember.squareChatMid);
    squareId = string(chat.squareMid); memberId = string(chatMember.squareMemberMid); state = ["1", "JOINED"].includes(String(chatMember.membershipState)) ? "JOINED" : "";
  } else if (["2", "NOTIFIED_JOIN_SQUARE_CHAT"].includes(type)) {
    const update = record(raw.notifiedJoinSquareChat); member = record(update.joinedMember); chatId = string(update.squareChatMid); state = "JOINED";
  } else if (["4", "NOTIFIED_LEAVE_SQUARE_CHAT"].includes(type)) {
    const update = record(raw.notifiedLeaveSquareChat); memberId = string(update.squareMemberMid); chatId = string(update.squareChatMid); state = "LEFT";
  } else if (["14", "NOTIFIED_UPDATE_SQUARE_CHAT_MEMBER"].includes(type)) {
    const update = record(raw.notifiedUpdateSquareChatMember), chatMember = record(update.squareChatMember);
    memberId = string(chatMember.squareMemberMid); chatId = string(update.squareChatMid) || string(chatMember.squareChatMid);
    state = ["1", "JOINED"].includes(String(chatMember.membershipState)) ? "JOINED" : ["2", "LEFT"].includes(String(chatMember.membershipState)) ? "LEFT" : "";
  } else return null;
  memberId ||= string(member.squareMemberMid); squareId ||= string(member.squareMid);
  if (scope === "square" && state !== "NAME") state = ({ "2": "JOINED", JOINED: "JOINED", "4": "LEFT", LEFT: "LEFT", "5": "KICK_OUT", KICK_OUT: "KICK_OUT", "6": "BANNED", BANNED: "BANNED" } as Record<string, string>)[String(member.membershipState)] ?? "";
  const created = timestamp(event.createdTime);
  if (!created || created < baselineBeforeMs) return null;
  if (!squareId && chatId && directory) squareId = (await directory.chat(chatId)).squareChat.squareMid;
  if (!state && ["11", "NOTIFIED_UPDATE_SQUARE_MEMBER"].includes(type) && string(member.displayName)) state = "NAME";
  if (!squareId || !memberId || !state || !created) return null;
  chatId ||= squareId;
  return { type: "memberChanged", eventId: `member:${squareId}:${scope === "square" ? "all" : chatId}:${memberId}:${state}:${created}`,
    squareId, chatId, memberId, displayName: name(member.displayName), scope, state, memberCreatedAtMs: timestamp(member.createdAt), createdAtMs: created,
    metadataJson: JSON.stringify({ source, eventType: type, receivedAtMs: Date.now() }) };
}

export async function normalizeEvents(event: Event, directory?: SquareDirectory, baselineBeforeMs = 0, source = "push"): Promise<CoreEvent[]> {
  if (["19", "NOTIFIED_KICKOUT_FROM_SQUARE"].includes(String(event.type))) {
    const payload = record(record(event.payload).notifiedKickoutFromSquare);
    if (!Array.isArray(payload.kickees) || payload.kickees.length > 64) throw new Error("InvalidKickEvent");
    if ((timestamp(event.createdTime) ?? 0) < baselineBeforeMs) return [];
    const result: CoreEvent[] = [];
    for (const target of payload.kickees) {
      const member = record(target);
      const converted = await normalizeEvent({ ...event, type: "NOTIFIED_UPDATE_SQUARE_MEMBER", payload: {
        notifiedUpdateSquareMember: { squareMid: string(member.squareMid), squareMemberMid: string(member.squareMemberMid),
          squareMember: { ...member, membershipState: "KICK_OUT" } },
      } } as unknown as Event, directory, baselineBeforeMs, source);
      if (converted?.type === "memberChanged") result.push({ ...converted, chatId: string(payload.squareChatMid) || converted.chatId,
        metadataJson: JSON.stringify({ source, eventType: String(event.type), receivedAtMs: Date.now() }) });
    }
    return result;
  }
  const single = await normalizeEvent(event, directory, baselineBeforeMs, source);
  return single ? [single] : [];
}
