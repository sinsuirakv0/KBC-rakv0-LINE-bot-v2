import type { BaseClient } from "@evex/linejs/base";
import type { OcRequest } from "../protocol/generated/OcRequest.js";
import { LINEStruct } from "@evex/linejs/thrift";
import type { OcResult } from "../protocol/generated/OcResult.js";
import type { OcMember } from "../protocol/generated/OcMember.js";
import type { OcChat } from "../protocol/generated/OcChat.js";
import type { MessageMention } from "../protocol/generated/MessageMention.js";

export function mentionMetadata(mention?: MessageMention | null): Record<string, string> | undefined {
  return mention ? { MENTION: JSON.stringify({ MENTIONEES: [{ S: String(mention.start), E: String(mention.end), M: mention.memberId }] }) } : undefined;
}

type ChatInfo = Awaited<ReturnType<BaseClient["square"]["getSquareChat"]>>;
type Member = Awaited<ReturnType<BaseClient["square"]["getSquareMember"]>>["squareMember"];
type DirectoryCursor = { syncToken: string; continuationToken: string; subscriptionId?: number };
// LINEJS自身の参加トーク列挙と同じ初期snapshotを使う。通常受信のcheckpointは変更しない。
export async function joinedChatPage(client: BaseClient, token?: string, limit = 100): Promise<{ chats: ChatInfo["squareChat"][]; continuationToken?: string }> {
  let cursor: DirectoryCursor | undefined;
  if (token) {
    if (token.length > 2048) throw new Error("DirectoryCursorLimit");
    cursor = JSON.parse(Buffer.from(token, "base64url").toString("utf8")) as DirectoryCursor;
    if (!cursor || typeof cursor.syncToken !== "string" || cursor.syncToken.length > 512
        || typeof cursor.continuationToken !== "string" || !cursor.continuationToken || cursor.continuationToken.length > 1024
        || (cursor.subscriptionId !== undefined && (!Number.isSafeInteger(cursor.subscriptionId) || cursor.subscriptionId <= 0))) throw new Error("InvalidDirectoryCursor");
  }
  const response = await client.square.fetchMyEvents({ syncToken: cursor?.syncToken ?? "", continuationToken: cursor?.continuationToken,
    subscriptionId: cursor?.subscriptionId, limit });
  if (!Array.isArray(response.events) || response.events.length > limit || typeof response.syncToken !== "string" || response.syncToken.length > 512) throw new Error("InvalidDirectoryPage");
  const chats = response.events.flatMap(event => {
    const chat = event.payload?.notifiedCreateSquareChatMember?.chat;
    if (!chat) return [];
    if (!/^m[0-9a-f]{8,63}$/i.test(chat.squareChatMid) || !/^s[0-9a-f]{8,63}$/i.test(chat.squareMid)) throw new Error("InvalidDirectoryChat");
    return [chat];
  });
  let continuationToken: string | undefined;
  if (response.continuationToken) {
    const subscriptionId = response.subscription?.subscriptionId == null ? cursor?.subscriptionId : Number(response.subscription.subscriptionId);
    if (subscriptionId !== undefined && (!Number.isSafeInteger(subscriptionId) || subscriptionId <= 0)) throw new Error("InvalidDirectorySubscription");
    continuationToken = Buffer.from(JSON.stringify({ syncToken: response.syncToken, continuationToken: response.continuationToken, subscriptionId })).toString("base64url");
    if (continuationToken.length > 2048) throw new Error("DirectoryCursorLimit");
  }
  return { chats, continuationToken };
}
const displayName = (value: string) => Array.from(value.replace(/[\r\n]/g, " ")).slice(0, 80).join("");
export function memberDto(member: Member): OcMember {
  if (!member || [member.squareMemberMid, member.squareMid].some(id => typeof id !== "string" || !id || id.length > 256)
      || !/^[0-9]{1,19}$/.test(String(member.revision))) throw new Error("InvalidSquareMemberResponse");
  return { memberId: member.squareMemberMid, squareId: member.squareMid, name: displayName(member.displayName ?? ""),
    role: String(member.role), state: String(member.membershipState), revision: String(member.revision) };
}

export class SquareDirectory {
  private cache = new Map<string, { at: number; value: ChatInfo }>();
  private pending = new Map<string, Promise<ChatInfo>>();
  constructor(private client: BaseClient) {}
  async chat(chatId: string): Promise<ChatInfo> {
    const cached = this.cache.get(chatId);
    if (cached && Date.now() - cached.at < 600_000) return cached.value;
    const pending = this.pending.get(chatId); if (pending) return pending;
    if (this.pending.size >= 32) throw new Error("SquareLookupFull");
    const request = this.client.square.getSquareChat({ squareChatMid: chatId }).then(value => {
      if (!value.squareChat?.squareMid || value.squareChat.squareChatMid !== chatId || !value.squareChatMember?.squareMemberMid) throw new Error("MissingSquareChatContext");
      if (this.cache.size >= 512 && !this.cache.has(chatId)) this.cache.delete(this.cache.keys().next().value!);
      this.cache.set(chatId, { at: Date.now(), value }); return value;
    }).finally(() => this.pending.delete(chatId));
    this.pending.set(chatId, request); return request;
  }
  async execute(chatId: string, request: OcRequest): Promise<OcResult> {
    const result: OcResult = { chats: [], members: [] };
    if (request.type === "reactions") {
      if (!["NICE", "LOVE"].includes(request.reactionType) || !request.messageId || !request.memberId) throw new Error("InvalidReactionRequest");
      let continuationToken: string | undefined;
      // 3.4.2はThrift定義だけを持つ。共通request経路で必要な一覧だけ最大400件確認する。
      for (let page = 0; page < 4; page++) {
        const response = await this.client.request.request(LINEStruct.SquareService_getMessageReactions_args({ request: {
          squareChatMid: chatId, messageId: request.messageId, type: request.reactionType === "NICE" ? "NICE" : "LOVE",
          limit: 100, continuationToken,
        } }), "getMessageReactions", this.client.square.protocolType, true, this.client.square.requestPath) as {
          reactions?: { type: string | number; reactor?: { squareMemberMid: string }; createdAt: unknown; updatedAt: unknown }[];
          continuationToken?: string;
        };
        if (!Array.isArray(response.reactions) || response.reactions.length > 100) throw new Error("InvalidReactionResponse");
        const own = response.reactions.find(reaction => reaction.reactor?.squareMemberMid === request.memberId);
        if (own) {
          const reactionType = ({ "2": "NICE", NICE: "NICE", "3": "LOVE", LOVE: "LOVE" } as Record<string, string>)[String(own.type)];
          const updatedAtMs = Number(own.updatedAt ?? own.createdAt);
          if (!reactionType || !Number.isSafeInteger(updatedAtMs) || updatedAtMs <= 0) throw new Error("InvalidReactionResponse");
          result.reaction = { memberId: request.memberId, reactionType, updatedAtMs };
          break;
        }
        if (!response.continuationToken) break;
        if (typeof response.continuationToken !== "string" || response.continuationToken.length > 2048 || response.continuationToken === continuationToken) throw new Error("InvalidReactionCursor");
        continuationToken = response.continuationToken;
      }
    } else if (request.type === "context") {
      const chat = await this.chat(chatId);
      const actor = await this.client.square.getSquareMember({ squareMemberMid: request.memberId });
      const botMemberId = chat.squareChatMember.squareMemberMid;
      let botRole = "未照会";
      let authority = "";
      if (request.authority) {
        botRole = String((await this.client.square.getSquareMember({ squareMemberMid: botMemberId })).squareMember.role);
        const response = await this.client.square.getSquareAuthority({ request: { squareMid: chat.squareChat.squareMid } });
        authority = Object.entries(response.authority ?? {}).filter(([key]) => key !== "revision" && key !== "squareMid")
          .slice(0, 16).map(([key, value]) => `${key}: ${String(value)}`).join("\n");
      }
      result.context = { squareId: chat.squareChat.squareMid, chatName: displayName(chat.squareChat.name ?? ""),
        botMemberId, botRole, actor: memberDto(actor.squareMember), authority };
    } else if (request.type === "member") {
      const member = (await this.client.square.getSquareMember({ squareMemberMid: request.memberId })).squareMember;
      result.member = memberDto(member);
      result.rawMemberName = member.displayName;
    } else if (request.type === "inspect") {
      if (!/^m[0-9a-f]{8,63}$/i.test(request.chatId) || request.memberIds.length > 2
          || request.memberIds.some(id => !/^p[0-9a-f]{8,63}$/i.test(id))) throw new Error("InvalidInspectionTarget");
      // 実験前はトークとBotの所属・roleを改めて照会し、以前の役割を根拠にしない。
      this.cache.delete(request.chatId);
      const chat = await this.chat(request.chatId);
      const bot = memberDto((await this.client.square.getSquareMember({ squareMemberMid: chat.squareChatMember.squareMemberMid })).squareMember);
      if (bot.memberId !== chat.squareChatMember.squareMemberMid || bot.squareId !== chat.squareChat.squareMid) throw new Error("InspectionBotScopeMismatch");
      result.context = { squareId: bot.squareId, chatName: displayName(chat.squareChat.name ?? ""), botMemberId: bot.memberId,
        botRole: bot.role, actor: bot, authority: "" };
      for (const id of request.memberIds) {
        const member = id === bot.memberId ? bot : memberDto((await this.client.square.getSquareMember({ squareMemberMid: id })).squareMember);
        if (member.memberId !== id || member.squareId !== bot.squareId) throw new Error("InspectionMemberScopeMismatch");
        result.members.push(member);
      }
    } else if (request.type === "chats") {
      const chats = new Map<string, OcChat>();
      const add = (chat: ChatInfo["squareChat"]) => {
        if (chat.squareMid === request.squareId && chats.size < 64) chats.set(chat.squareChatMid, { chatId: chat.squareChatMid,
          name: displayName(chat.name ?? chat.squareChatMid), squareId: chat.squareMid, isMain: ["4", "SQUARE_DEFAULT"].includes(String(chat.type)) });
      };
      add((await this.chat(chatId)).squareChat);
      let continuationToken: string | undefined;
      // SDK未実装・一部ページ失敗でも現在トークを候補として残す。常時巡回には使わない。
      try {
        for (let page = 0; page < 4; page++) {
          const response = await joinedChatPage(this.client, continuationToken);
          for (const chat of response.chats ?? []) add(chat);
          if (!response.continuationToken || response.continuationToken === continuationToken) break;
          continuationToken = response.continuationToken;
        }
      } catch { /* 未取得のトークでは、そのトークから直接設定できる。 */ }
      try {
        const response = await this.client.livetalk.getSquareInfoByChatMid({ request: { squareChatMid: chatId } });
        if (response.defaultChatMid) add((await this.chat(response.defaultChatMid)).squareChat);
      } catch { /* 本OCの手動選択を保持する。 */ }
      result.chats = [...chats.values()].sort((a, b) => Number(b.isMain) - Number(a.isMain) || a.name.localeCompare(b.name));
    } else if (request.type === "joinedChats") {
      const response = await joinedChatPage(this.client, request.continuationToken ?? undefined, 30);
      if (!Array.isArray(response.chats) || response.chats.length > 30) throw new Error("InvalidJoinedChatPage");
      result.chats = response.chats.map(chat => {
        if (!chat.squareChatMid || !chat.squareMid) throw new Error("InvalidJoinedChat");
        return { chatId: chat.squareChatMid, squareId: chat.squareMid, name: displayName(chat.name ?? ""),
          isMain: ["4", "SQUARE_DEFAULT"].includes(String(chat.type)) };
      });
      result.continuationToken = response.continuationToken || undefined;
    } else if (request.type === "members") {
      if (!["JOINED", "LEFT", "KICK_OUT", "BANNED"].includes(request.state)) throw new Error("InvalidMemberSearchState");
      const response = await this.client.square.searchSquareMembers({ request: { squareMid: request.squareId,
        searchOption: { membershipState: request.state as "JOINED" | "LEFT" | "KICK_OUT" | "BANNED", displayName: request.query,
          memberRoles: [], ableToReceiveMessage: "NONE", ableToReceiveFriendRequest: "NONE", chatMidToExcludeMembers: "",
          includingMe: true, excludeBlockedMembers: false, includingMeOnlyMatch: false },
        limit: 20, continuationToken: request.continuationToken ?? undefined } });
      if (!Array.isArray(response.members) || response.members.length > 20) throw new Error("InvalidMemberSearchPage");
      result.members = response.members.map(memberDto);
      if (result.members.some(member => member.squareId !== request.squareId)) throw new Error("MemberSearchScopeMismatch");
      result.continuationToken = response.continuationToken || undefined;
    } else if (request.type === "membership") {
      if (!["BANNED", "KICK_OUT"].includes(request.state)) throw new Error("InvalidMembershipState");
      const response = await this.client.square.updateSquareMember({ request: { updatedAttrs: [5], updatedPreferenceAttrs: [],
        squareMember: { squareMemberMid: request.memberId, squareMid: request.squareId, revision: BigInt(request.revision), membershipState: request.state as "BANNED" | "KICK_OUT" } } });
      result.member = memberDto(response.squareMember);
      if (result.member.memberId !== request.memberId || result.member.squareId !== request.squareId) throw new Error("UnconfirmedMembershipTarget");
      if (![request.state, request.state === "BANNED" ? "6" : "5"].includes(result.member.state)) throw new Error("UnconfirmedMembershipChange");
    } else if (request.type === "profile") {
      if (!request.name || !/^[0-9]{1,19}$/.test(request.revision)) throw new Error("InvalidBotName");
      const chat = await this.chat(chatId);
      // 実行OCのBot自身の表示名だけを更新する。他人・他OC・roleは対象にしない。
      if (chat.squareChat.squareMid !== request.squareId || chat.squareChatMember.squareMemberMid !== request.memberId) throw new Error("BotProfileScopeMismatch");
      await this.client.square.updateSquareMember({ request: { updatedAttrs: ["DISPLAY_NAME"], updatedPreferenceAttrs: [],
        squareMember: { squareMemberMid: request.memberId, squareMid: request.squareId, revision: BigInt(request.revision), displayName: request.name } } });
      // 更新応答は完全なプロフィールとは限らないため、1回の読み取りで実際の変更を確認する。
      const response = await this.client.square.getSquareMember({ squareMemberMid: request.memberId });
      result.member = memberDto(response.squareMember);
      if (result.member.memberId !== request.memberId || result.member.squareId !== request.squareId
          || response.squareMember.displayName !== request.name || !["JOINED", "2"].includes(result.member.state)) throw new Error("UnconfirmedBotNameChange");
    } else if (request.type === "roles") {
      if (!/^s[0-9a-f]{8,63}$/i.test(request.squareId) || request.members.length < 1 || request.members.length > 2
          || new Set(request.members.map(member => member.memberId)).size !== request.members.length
          || request.members.some(member => member.squareId !== request.squareId || !/^p[0-9a-f]{8,63}$/i.test(member.memberId)
            || !/^[0-9]{1,19}$/.test(member.revision) || !["ADMIN", "CO_ADMIN", "MEMBER"].includes(member.role))) throw new Error("InvalidRoleUpdate");
      const response = await this.client.square.updateSquareMembers({ request: { updatedAttrs: ["ROLE"],
        members: request.members.map(member => ({ squareMemberMid: member.memberId, squareMid: member.squareId,
          revision: BigInt(member.revision), role: member.role as "ADMIN" | "CO_ADMIN" | "MEMBER" })) } });
      const responseMembers = Object.values(response.members ?? {});
      if (responseMembers.length > 8) throw new Error("InvalidRoleUpdateResponse");
      const updated = responseMembers.map(memberDto);
      for (const expected of request.members) {
        const member = updated.find(member => member.memberId === expected.memberId && member.squareId === expected.squareId);
        const numericRole = { ADMIN: "1", CO_ADMIN: "2", MEMBER: "10" }[expected.role];
        if (!member || ![expected.role, numericRole].includes(member.role)) throw new Error("UnconfirmedRoleChange");
        result.members.push(member);
      }
    } else if (request.type === "post") {
      const sent = await this.client.square.sendMessage({ squareChatMid: request.chatId, text: request.text,
        contentMetadata: mentionMetadata(request.mention) });
      result.messageId = sent.createdSquareMessage?.message?.id;
      if (!result.messageId) throw new Error("MissingSentMessageId");
    } else if (request.type === "delete") {
      await this.client.square.destroyMessage({ squareChatMid: request.chatId, messageId: request.messageId });
    } else if (request.type === "report") {
      await this.client.square.reportSquareMessage({ request: { squareMid: request.squareId, squareChatMid: chatId,
        squareMessageId: request.messageId, reportType: "SCAM" } });
    }
    return result;
  }
}
