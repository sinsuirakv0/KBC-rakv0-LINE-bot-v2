# OCでのBotの表示名変更

2026-10-04。利用者指定でBOT管理者専用。実装とオフライン検証の対象。実LINEの表示変更は配備後の利用者試験で確認する。

## 入力・対象・権限

`!bot name 名前` と `o.bot name 名前`。実行したトークが属するOCのBot自身のプロフィールを変更する。OC名やアカウント全体の名前は変更しない。別OCでは別のプロフィールを持ち、同じOCのサブトークは共通のプロフィールを使う。[LINE公式のプロフィール説明](https://help.line.me/line/smartphone?contentId=20005375&lang=ja)、[サブトークの説明](https://help.line.me/line/android/sp?contentId=20024745&lang=ja)。

現在のOC所属を確認後、共通bot_rankでBOT admin以上だけを許可する。OCのADMIN / CO_ADMINやBOT modだけでは実行できない。権限は既存の機密permissionsファイルを参照する。BotがOCの一般メンバーでも、自分のプロフィール更新をAPIへ依頼できる。サーバー側で拒否された場合は結果を報告する。

名前の前後の空白を除き、途中の空白・日本語・絵文字を保つ。空文字・制御文字・改行を拒否する。運用上の入力上限は20 UTF-16単位とし、LINE公式の文字数仕様としては扱わない。名前を分割して複数更新したり、入力を黙って切り詰めたりしない。`!bot` / `!bot help` / `!help bot` は [共通txt案内](../../../../../content/help/bot.txt) を返す。

## 関数と既存基盤の関係

| 関数 | 役割・後続 |
| --- | --- |
| bot::parse | prefixとname引数を識別し、名前本文をInputへ保存 |
| oc::ingest / complete | 既存Context照会で実行者と所属を確認し、bot::executeへ渡す |
| bot::execute | BOT管理者と入力を確認し、Member照会へ進む |
| bot::target | Bot自身・OC・JOINED・現在名を確認。変更不要なら更新しない。再度BOT権限を確認し、Profile変更Actionを発行 |
| SquareDirectory.execute(profile) | 実行トークの親OCとBot MIDを照合し、updateSquareMemberへ渡す。返されたMID・OC・名前・JOINEDを確認して成功を返す |
| bot::mutation | 共通oc_historyへ旧名・新名・結果を記録し、実行トークへ通常送信で報告 |

Protocol v9のProfileは変更要求。LINEJS 3.4.2のupdateSquareMemberでupdatedAttrsはDISPLAY_NAME（数値1）だけ、updatedPreferenceAttrsは空。Bot MID・OC MID・現在revision・新しいdisplayNameを渡し、role・membershipStateを更新しない。旧Botのsrc/runtime/ocProfileStatus.tsが使用していたAPIを参照した。旧版の自動状態suffixや定期巡回は移植しない。2026-10-04にnpmの最新公開版3.4.2を再確認し、依存lockの変更は不要だった。

専用Queue・Cache・Table・定期取得は追加しない。既存のOC Outbox、照会Worker、有限配送と共通API制御を共有する。受付・権限確認の60秒期限、変更配送の30秒期限、fetch直前のsending記録を引き継ぐ。通信後の不明結果はunknownとして残し、自動再更新しない。再起動後もLINE側のプロフィールが正とし、古い保存名を再適用しない。

## 検証範囲

smoke:ocでOC管理人・副官・BOT mod・一般参加者の拒否、BOT管理者の成功、サブトークの親OC解決、日本語・空白・絵文字、入力上限、同名の更新省略、roleとmembershipStateの非変更、通信後unknownの非再実行を確認した。smoke:commandsで共通一覧・helpへの登録を確認した。npm run build、smoke:oc、smoke:commands、既存smoke、cargo clippy --workspace --all-targets --release --locked -- -D warnings、BOM / LF / 資料リンク検査は通過。API契約はモック検証であり、実LINEでの改名成功・禁止文字の挙動とは区別する。
