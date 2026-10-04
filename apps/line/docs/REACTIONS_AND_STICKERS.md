# OCのリアクションとスタンプ

2026-10-04。LINEJS 3.4.2（npm revision 11）と同梱linejs-typesを確認。npmの最新公開版も3.4.2。以下は実装確認とオフライン検証。15:54 JSTの本環境healthでNOTIFICATION_MESSAGE_REACTIONを1件観測したが、検索promptへの操作との対応・照会APIの成功・本文絵文字の実表示・スタンプ送信成功は未確認。

## 現行のページ操作

2026-10-04、利用者の「リアクションは無理そう」という実運用報告と指定で、リアクション方式を廃止した。検索・関連ファイルの一覧は1ページ10件。「次」「前」または「3p」を最新一覧へリプライする。1〜10は項目選択、9/0はページ移動に使わない。本人・トーク・prompt・期限の確認と、送信成功まで旧ページを保持する仕組みは維持する。[現行仕様・関数](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

Coreはリアクション通知から照会やページ投稿を作らない。SquareDirectoryは旧snapshotのReactions要求をAPIなしで完了する。Protocol v12の型は保存互換のため残すが、案内のLINE絵文字装飾は付けない。!id sticker / stamp / emojiは継続する。報告からLINE全体のリアクションAPIが利用不能だとは断定せず、今回の採用を取り消した判断として残す。

## 旧ページ操作の調査（廃止）

以下は廃止前の実装・模擬検証の記録であり、現行仕様ではない。

利用者指定で検索一覧の👍（いいね）を次ページ、❤️（ハート）を前ページにした。項目選択は番号リプライ。ut / tut / stと関連ファイル選択が同じSessionを使う。OCでは標準以外の絵文字リアクションを使えないという[公式の制約](https://help.line.me/line/?contentId=20020725&lang=ja)がある。

案内は「一覧を長押ししてリアクションを付ける」と明示する。本文への絵文字リプライはページ操作として処理しない。2026-10-04に[公式のLINE絵文字一覧](https://developers.line.biz/ja/docs/messaging-api/emoji-list/)を展開し、標準セット670e0cce840a8236ddd4ee4cの143（いいね）・165（ハート）の画像を目視照合した。これらを本文の案内に使う。本文に置く絵文字は操作ボタンではなく、長押しメニューのリアクションを示す画像である。

Protocol v12のSendMessage.emojis / MessageEmojiはproductId・emojiIdとUTF-16開始終了位置だけを渡す。commands::message_emojisは分割後の案内の該当部分を最大20個装飾する。AdapterのmessageMetadataはmentionMetadataを維持し、SDK EmojiMetaと同じREPLACE.sticon.resources（S / E / productId / sticonId / version=1 / resourceType=STATIC）へ変換する。常時APIを増やさない。Squareの実送信での表示は未確認で、公式Messaging APIのemojisフィールドをSquareへ直接渡さない。

3.4.2のSquareEventType 47（NOTIFICATION_MESSAGE_REACTION）はnotificationMessageReactionにトークMID・messageId・type・reactorNameを持つが、reactor MIDは持たない。NICE=2、LOVE=3、UNDO=1。通知の表示名を本人確認に使わない。normalizeEventはNICE / LOVEだけをProtocol v11のReactionNotifiedに変換し、時刻・トーク・メッセージ・種類で重複照合する。取り消しとその他の種類はページを変えない。baseline以前の通知も使わない。

Coreのrequest_reactionは期限内の最新promptを検索し、移動できる方向だけOcRequest::Reactionsを既存Outboxへ登録する。照会・配送中は同じ一覧の通知を集約する。getMessageReactionsは3.4.2のSquareServiceに高水準methodがないが、LINEStruct.SquareService_getMessageReactions_argsと返値の型は同梱されている。SquareDirectory.executeからSDKの共通request.requestへ、SquareのprotocolType / requestPathで呼び出す。受信正規化中は照会せず、既存の1照会Worker・全API最大2並列/250ms/cooldownを使う。

取得は要求種類を100件×最大4ページに限定し、検索者のsquareMemberMidだけを返す。MID・種類と時刻をcomplete_reactionで照合する。時刻はprompt送信時刻以降を原則とし、サーバーと端末の差に30秒の余裕を持つ。他人だけのリアクション、古いprompt、別トーク、期限切れ、ページ端は一覧を更新しない。サーバーでこのAPIが使えない、400件以内に検索者がいない、通知自体が届かない場合は移動できない。無条件に数字へ自動フォールバックする実装は追加しない。

変更先はpending_payloadに置き、現行payload / promptは新一覧の送信成功まで残す。通信前の確定失敗は旧一覧へ戻し、通信後unknownは再送せず照合待ちにする。送信成功後に旧一覧を管理者削除し、削除失敗でも新しいpromptを維持する。本文は96 UTF-16単位の候補8件に短縮し、操作案内が別投稿へ分割されないようにする。詳細と関数は[Command仕様](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

smoke:commandsは本SDKのThrift引数を通した照会、別人・別トーク・旧prompt・連打、切替中の番号、確定失敗後の復帰、再起動、ページ選択を通信なしで確認する。実通知47の到達と実getMessageReactionsの返値は、配備後に利用者の少数OC操作で確認する。

v12のオフライン検証では、案内のUTF-16範囲・通常配送がSDKへ渡すREPLACE・絵文字リプライの無視を確認。smoke:ocでsticker / stamp・複数emoji・本文に添えるemoji・明示message ID・同OCサブトーク・別OC参照拒否・不正JSON・20個上限・再起動後の保持を既存経路に追加した。型検査・Native build・3種類のSmokeを通過した。模擬SDK送信を実LINE表示の成功として扱わない。

## スタンプ

LINEJSのStickerMetadataはSTKPKGID（セットID）、STKID（スタンプID）、STKVER（version）、STKTXT（代替文）、任意のSTKOPTを定義している。受信したcontentType=STICKERのcontentMetadataに入る。normalizeEventはmetadataJsonへ保存し、既存トークログも保持する。!id sticker / stampへ専用表示を追加した。[参照範囲・関数・上限](../../../crates/kbc-core/src/oc/docs/ID.md)。

LINE絵文字はcontentMetadata.REPLACEのJSON内にsticon.resourcesがあり、productIdがセットID、sticonIdが絵文字ID。SDK collectEmojiURLs / getTextDecorationsも同じ項目を読む。!id emojiは先頭20個まで表示し、壊れたREPLACEや対象外の情報は表示に使わない。IDの受信保存は新規投稿から開始し、既存本文ログの全件走査や過去API取得は行わない。

SquareService.sendMessageはcontentType / contentMetadataを指定できるため、STICKER形式と該当metadataを渡す送信経路は存在する。画像をダウンロードして添付する方式とは異なる。SquareMessage.getStickerURLはSTKIDとSTKOPTから静止PNGまたはアニメーションPNGのURLを組み立てるが、画像の取得成功はアカウントによるスタンプ送信可否を保証しない。

このBotはLINEJSのSquare（通常LINEアカウント）を使う。公式Messaging APIの[packageId / stickerId仕様](https://developers.line.biz/en/docs/messaging-api/sticker-list/)はIDの概念を説明する一次資料だが、公式アカウントAPIの送信可能リストをSquareへそのまま適用しない。IDさえあれば購入・所有・公開状態を問わず何でも送れるという根拠はない。受信済みmetadataを参照して少数で試すため、BOT管理者専用の[!test sticker](../../../crates/kbc-core/src/oc/docs/TEST_STICKER.md)を追加した。STKPKGID / STKID / STKVER / STKTXT / 任意STKOPTを既存sendMessageへ渡す。所有条件・実APIの受理・端末表示は未確認で、本文の標準絵文字装飾は現在廃止している。

参照: [LINEJS SquareService実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/service/square/mod.ts)、[SquareMessage実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/client/features/message/square.ts)。実際の採用版はlockとnode_modulesの3.4.2配布物で照合した。
