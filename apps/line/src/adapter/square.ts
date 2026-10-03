import type { BaseClient } from "@evex/linejs/base";
import type { OcRequest } from "../protocol/generated/OcRequest.js";
import type { OcResult } from "../protocol/generated/OcResult.js";
import type { OcMember } from "../protocol/generated/OcMember.js";
import type { OcChat } from "../protocol/generated/OcChat.js";

type ChatInfo = Awaited<ReturnType<BaseClient["square"]["getSquareChat"]>>;
type Member = Awaited<ReturnType<BaseClient["square"]["getSquareMember"]>>["squareMember"];
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
    if (request.type === "context") {
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
      result.member = memberDto((await this.client.square.getSquareMember({ squareMemberMid: request.memberId })).squareMember);
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
          const response = await this.client.square.getJoinedSquareChats({ request: { limit: 100, continuationToken } });
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
      const response = await this.client.square.getJoinedSquareChats({ request: { limit: 30, continuationToken: request.continuationToken ?? undefined } });
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
    } else {
      await this.client.square.reportSquareMessage({ request: { squareMid: request.squareId, squareChatMid: chatId,
        squareMessageId: request.messageId, reportType: "SCAM" } });
    }
    return result;
  }
}
