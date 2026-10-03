# リプライ送信テスト

2026-10-04。利用者の指定で通常のreplyコマンドではなく、!test replyとして追加。実LINEのサブトーク間・別OCでの表示は配備後の試験対象。

!test reply <メッセージID> <本文> は現在トークへ送信する。メッセージIDの直後へ --to <トークMID> を付けると送信先を指定できる。o.testも同じ。送信先はmで始まるトークMIDで、sで始まるOC MIDやpで始まるユーザーMIDを使わない。返信先のIDは数字1〜32文字。ID情報の取得だけを行う!id replyとは別の操作。

実行OCまたは実行トークのBOT admin権限だけを許可する。OC ADMIN / CO_ADMIN、BOT modでは実行できない。共通Context照会で実行者のOC・member ID・JOINEDと受付期限60秒を確認してから送信を登録する。送信先に対する管理者権限は要求しない。任意先への送信を許可する診断操作として、利用者がBOT管理者限定を指定した。

--toを省略すると現在トーク、指定時は同じOCのサブトーク・別OCも試せる。返信元が受信索引にない場合も拒否しない。LINEJS 3.4.2のsquare.sendMessageはrelatedMessageIdを指定するとSQUARE / REPLYを設定する。返信元トークMIDはSDKのこの要求に含まれない。別OCの参照が実LINEで許可されることは未確認であり、APIが受け付けるかを調べるためのテストとする。BOTが送信できるトークを明示する。

本文の改行・連続空白を保ち、1〜1,500 UTF-16単位まで。先頭を--toという本文にしたい場合は、本文の前に--を置く。長い本文を分割すると同じ参照先へ何度も投稿するため、上限を超えたら案内だけを返す。成功時は指定先への1投稿だけで、受付案内や成功案内を追加しない。

| 関数 | 役割と接続 |
| --- | --- |
| test_reply::parse / take_word | !test replyを識別し、本文全体をJobへ残す。helpは既存catalogへ渡す |
| test_reply::execute | BOT権限・ID・MID・本文上限を検査し、共通text_actionへ返信先IDと送信先を渡す |
| oc::ingest / complete | 既存OC照会・Jobの永続受付とContext結果からexecuteを呼ぶ |
| text_action / TextDelivery | 任意のrelated_message_idを既存SendMessageへ設定。通常応答は省略し、副官通知も同じ設定を使う |
| deliverAction / ApiScheduler | 既存2配送Worker・全API枠で送る。実transport直前にsendingを保存 |

Protocol v7の既存SendMessageだけで表現できるのでProtocol変更・専用Queue・HTTP・Cache・Timerは追加しない。実通信後の例外は共通契約どおりunknownとして保存し、自動再投稿しない。配送結果は既存deliveryログとhealthのfailed / unknownで確認する。生のSDK例外を別OCへ投稿しない。

検証は既存smoke:ocへ、BOT adminのみの許可、通常トーク・別送信先、ID誤指定、UTF-16上限、改行保持、SDKへ渡るrelatedMessageIdを追加する。通信なしのmock検証を実LINEでの別OC返信の成功と読み替えない。[IDの情報取得](ID.md)、[共通Runtime](../../../docs/RUNTIME.md)。

同日、npm run build、smoke / smoke:commands / smoke:oc、cargo clippy --workspace --all-targets --release --locked -- -D warningsが通過。OC ADMINとBOT modの拒否、BOT adminの1投稿、o.testの別名、別送信先へのMID転送、本文の改行・連続空白保持、ID/MIDの混同拒否、絵文字を含むUTF-16上限、helpの共通経路を確認した。実LINEへのテスト投稿は行っていない。
