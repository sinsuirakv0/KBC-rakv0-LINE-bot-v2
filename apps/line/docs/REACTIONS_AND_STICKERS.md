# OCのリアクションとスタンプ

2026-10-04。LINEJS 3.4.2（npm revision 11）と同梱linejs-typesを確認。npmの最新公開版も3.4.2。以下は実装確認とオフライン検証であり、実OCの通知到達・スタンプ送信成功は未確認。

## ページ操作

利用者指定で検索一覧の👍（いいね）を次ページ、❤️（ハート）を前ページにした。項目選択は番号リプライ。ut / tut / stと関連ファイル選択が同じSessionを使う。OCでは標準以外の絵文字リアクションを使えないという[公式の制約](https://help.line.me/line/?contentId=20020725&lang=ja)がある。

3.4.2のSquareEventType 47（NOTIFICATION_MESSAGE_REACTION）はnotificationMessageReactionにトークMID・messageId・type・reactorNameを持つが、reactor MIDは持たない。NICE=2、LOVE=3、UNDO=1。通知の表示名を本人確認に使わない。normalizeEventはNICE / LOVEだけをProtocol v11のReactionNotifiedに変換し、時刻・トーク・メッセージ・種類で重複照合する。取り消しとその他の種類はページを変えない。baseline以前の通知も使わない。

Coreのrequest_reactionは期限内の最新promptを検索し、移動できる方向だけOcRequest::Reactionsを既存Outboxへ登録する。照会・配送中は同じ一覧の通知を集約する。getMessageReactionsは3.4.2のSquareServiceに高水準methodがないが、LINEStruct.SquareService_getMessageReactions_argsと返値の型は同梱されている。SquareDirectory.executeからSDKの共通request.requestへ、SquareのprotocolType / requestPathで呼び出す。受信正規化中は照会せず、既存の1照会Worker・全API最大2並列/250ms/cooldownを使う。

取得は要求種類を100件×最大4ページに限定し、検索者のsquareMemberMidだけを返す。MID・種類と時刻をcomplete_reactionで照合する。時刻はprompt送信時刻以降を原則とし、サーバーと端末の差に30秒の余裕を持つ。他人だけのリアクション、古いprompt、別トーク、期限切れ、ページ端は一覧を更新しない。サーバーでこのAPIが使えない、400件以内に検索者がいない、通知自体が届かない場合は移動できない。無条件に数字へ自動フォールバックする実装は追加しない。

変更先はpending_payloadに置き、現行payload / promptは新一覧の送信成功まで残す。通信前の確定失敗は旧一覧へ戻し、通信後unknownは再送せず照合待ちにする。送信成功後に旧一覧を管理者削除し、削除失敗でも新しいpromptを維持する。本文は96 UTF-16単位の候補8件に短縮し、操作案内が別投稿へ分割されないようにする。詳細と関数は[Command仕様](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

smoke:commandsは本SDKのThrift引数を通した照会、別人・別トーク・旧prompt・連打、切替中の番号、確定失敗後の復帰、再起動、ページ選択を通信なしで確認する。実通知47の到達と実getMessageReactionsの返値は、配備後に利用者の少数OC操作で確認する。

## スタンプ

LINEJSのStickerMetadataはSTKPKGID（セットID）、STKID（スタンプID）、STKVER（version）、STKTXT（代替文）、任意のSTKOPTを定義している。受信したcontentType=STICKERのcontentMetadataに入る。現在のnormalizeEventはこれらをmetadataJsonへ保存し、既存トークログも保持する。既存!idはスタンプ専用IDの整形表示をまだ持たない。

SquareService.sendMessageはcontentType / contentMetadataを指定できるため、STICKER形式と該当metadataを渡す送信経路は存在する。画像をダウンロードして添付する方式とは異なる。SquareMessage.getStickerURLはSTKIDとSTKOPTから静止PNGまたはアニメーションPNGのURLを組み立てるが、画像の取得成功はアカウントによるスタンプ送信可否を保証しない。

このBotはLINEJSのSquare（通常LINEアカウント）を使う。公式Messaging APIの[packageId / stickerId仕様](https://developers.line.biz/en/docs/messaging-api/sticker-list/)はIDの概念を説明する一次資料だが、公式アカウントAPIの送信可能リストをSquareへそのまま適用しない。IDさえあれば購入・所有・公開状態を問わず何でも送れるという根拠はない。実装する場合は受信済みmetadataを確認し、Botアカウントが利用可能な少数のスタンプで検証する。今回のスタンプ作業は調査と資料化のみ。

参照: [LINEJS SquareService実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/service/square/mod.ts)、[SquareMessage実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/client/features/message/square.ts)。実際の採用版はlockとnode_modulesの3.4.2配布物で照合した。
