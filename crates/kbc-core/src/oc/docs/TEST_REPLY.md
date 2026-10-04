# リプライ送信テスト

2026-10-04。利用者の指定で通常のreplyコマンドではなく、!test replyとして追加。続く仕様訂正で、送信先を実行トークに固定し、指定MIDを返信元トークへ変更した。配備後に利用者から実リプライ表示の成功を報告。トークの組合せと通知の条件は未確定。

!test reply <メッセージID> <本文> はコマンドを実行したトークへ送信する。メッセージIDの直後へ --chat <返信元トークMID> を付けると、そのメッセージがあるトークを指定できる。o.testも同じ。MIDはmで始まるトークMIDで、sで始まるOC MIDやpで始まるユーザーMIDを使わない。返信先のIDは数字1〜32文字。ID情報の取得だけを行う!id replyとは別の操作。旧--toは案内だけを返し、リプライ投稿を登録しない。

実行OCまたは実行トークのBOT admin権限だけを許可する。OC ADMIN / CO_ADMIN、BOT modでは実行できない。共通Context照会で実行者のOC・member ID・JOINEDと受付期限60秒を確認してから送信を登録する。返信元のOCでの管理権限は要求せず、指定MIDから送信先を切り替えない。

--chatの返信元は同じトーク・同OCのサブトーク・別OCを指定でき、投稿先はいずれも実行トーク。LINEJS 3.4.2のsquare.sendMessageはrelatedMessageIdを指定するとSQUARE / REPLYを設定し、squareChatMid / message.toは投稿先を表す。返信元トークMIDの専用引数はない。--chatは直近48時間・最大8,192件のmessage_refsと照合し、指定トークにそのIDがなく、別トークにあることを観測済みなら拒否する。未観測・期限外のIDは実験用として許可する。省略時はIDだけで指定し、返信元を実行トークと推定しない。照合のためにLINE履歴の追加取得や常駐索引を増やさない。別OCの参照が実LINEで許可されることは未確認。2026-10-04にnpm公開版3.4.2と手元配布物の実装を再確認した。

本文の改行・連続空白を保ち、1〜1,500 UTF-16単位まで。先頭を--chat等という本文にしたい場合は、本文の前に--を置く。長い本文を分割すると同じ参照先へ何度も投稿するため、上限を超えたら案内だけを返す。成功時は実行トークへの1投稿だけで、受付案内や成功案内を追加しない。

| 関数 | 役割と接続 |
| --- | --- |
| test_reply::parse / take_word | !test replyを識別し、本文全体をJobへ残す。helpは既存catalogへ渡す |
| test_reply::execute | BOT権限・ID・返信元MID・本文上限を検査し、既存message_refsと照合。共通text_actionへ返信先IDと実行トークを渡す |
| oc::ingest / complete | 既存OC照会・Jobの永続受付とContext結果からexecuteを呼ぶ |
| text_action / TextDelivery | 任意のrelated_message_idを既存SendMessageへ設定。通常応答は省略し、副官通知も同じ設定を使う |
| deliverAction / ApiScheduler | 既存2配送Worker・全API枠で送る。実transport直前にsendingを保存 |

Protocol v7の既存SendMessageだけで表現できるのでProtocol変更・専用Queue・HTTP・Cache・Timerは追加しない。実通信後の例外は共通契約どおりunknownとして保存し、自動再投稿しない。配送結果は既存deliveryログとhealthのfailed / unknownで確認する。生のSDK例外を別OCへ投稿しない。

既存smoke:ocで、BOT adminのみの許可、返信元がサブトーク・別OC・未観測でも実行トークへ送ること、ID/返信元MIDの不一致、旧--toの拒否、UTF-16上限、改行保持、SDKへ渡るrelatedMessageIdを確認する。通信なしのmock検証を実LINEでの別OC返信の成功と読み替えない。[IDの情報取得](ID.md)、[共通Runtime](../../../docs/RUNTIME.md)。

返信元MIDへ訂正した版でnpm run build、smoke / smoke:commands / smoke:oc、cargo clippy --workspace --all-targets --release --locked -- -D warningsが通過。OC ADMIN / BOT modの拒否、BOT adminの1投稿、o.testの別名、サブトーク・別OCの観測済みIDと未観測IDでも投稿先が実行トークであること、返信元MIDの不一致・旧--toの拒否、本文の改行・空白保持、ID/MIDの混同拒否、絵文字を含むUTF-16上限、helpの共通経路を確認した。実LINEへのテスト投稿は行っていない。

## 利用者による実LINE確認

同日、3dd8c61の配備後に利用者から「リプライできたが通知は鳴らない」と報告。表示の成功と端末通知を分けて扱う。同じトーク・サブトーク間・別OCのどの組合せか、端末と通知設定はまだ確認しておらず、別OC全般の成功や通知不具合と断定しない。Codexから追加の実投稿は行っていない。

[LINE公式のOC参加者向けヘルプ](https://help.line.me/line/smartphone?contentId=20005375&lang=ja)は、自分の投稿がリプライされたときの通知を設定できないと案内する。これを今回の端末通知の原因確定とは扱わない。メンションには別の通知設定があり、表示・MENTION metadata・相手への通知は別に確認する。[公式の通知設定](https://help.line.me/line/smartphone/?contentId=20011380)。現行!test replyはメンションを自動付与しない。
