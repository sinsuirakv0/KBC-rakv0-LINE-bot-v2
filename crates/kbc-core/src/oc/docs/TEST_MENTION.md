# 自由な表示名・複数人メンションの試験

2026-10-05。BOT管理者専用の `!test mention-label` を追加。目的は本文の表示名と通知先を分け、一個の表示範囲から複数人へ通知できるか実機で検証すること。Discordのロール相当の保存・自動配信は実装していない。

## 入力

`!test mention-label <pMID> <pMID> ... [--target-chat <mMID>] [--separate] [--apply] -- 表示名`

pMIDは1〜9人、重複・英大文字の16進部分は正規化してまとめる。MIDは全てflagより前、表示名は最後の--以後。表示名は1〜100 UTF-16単位、前後の空白は除く。内部の空白・改行・絵文字を保持し、先頭に@を付けて送信する。一人だけ指定すれば表示名を変更した単独メンションになる。引数不正・上限超過は照会や投稿前に拒否する。

既定では `@表示名` を一個だけ投稿し、全対象へ同じS/Eを指定する。`--separate` は比較用で、同じ表示名を半角空白で区切って人数分並べ、各MIDへ独立した範囲を割り当てる。本文・範囲はUTF-16で計算し、本文は1,500単位以下の1投稿とする。

BOT adminだけが実行できる。従来のtest mentionと同じく実行OC・照会先OCの両方を `!test allow <sMID> ...` で登録し、現参加状態・所属を照合する。--target-chatはメンバーの所属確認先であり、投稿先は常に実行トーク。異なるOCのメンバーを指定した場合も同じ検証範囲内で試し、対象所属を混ぜない。--applyなしは未実行の確認表示で、メンションmetadataは送信しない。

`!help test-mention` は専用txtの案内。!helpのindexへは自動追加しない。previewは指定名・方式・人数・実行先を表示し、結果はAPIの成否と送信message IDを既存通知で返す。

## 確認した仕組みと未確認点

採用LINEJS 3.4.2の `client/features/message/utils.ts` は、MENTION.MENTIONEESへ `{S:文字範囲の開始,E:終了,M:MID}` を追加する。本文e.textと対象midは独立し、allの場合だけA=1を使う。旧LINE main.tsのmentionMetadataも対象配列からこの形式を作っていた。2026-10-05にnpm公開版3.4.2を再確認し、依存lockは変更していない。

同じS/Eを複数entryで共有したときのサーバー・端末の扱いはSDKの生成例からは確定しない。Mに配列・カンマ区切りを入れず、各entryには一つのpMIDを指定する。全員通知のA=1は使わない。APIの受理、表示名が維持されるか、メンションのタップ先、対象全員への端末通知を別々に確認する。API成功を通知成功と記録しない。通知設定や端末による差も実機の記録へ含める。

## 関数とProtocol

| 関数・型 | 責務と接続 |
| --- | --- |
| test::parse / label_plan | 専用subcommand、MID・flag・表示名を解析し、有限なPlanへ保存 |
| test::execute / inspected | 既存BOT権限・allow・60秒期限・Inspectを共有し、全メンバーの所属・JOINEDを検査。同範囲または個別範囲のPostを一件だけ登録 |
| MessageMention / MentionTarget | Protocol v18。既存のmemberId/start/endを維持し、任意additionalで残り最大8人分のMID・位置を運ぶ。旧保存DTOはadditionalなしで復元 |
| SquareDirectory.execute / mentionMetadata | Inspectを最大9人に広げ、同じ既存API枠で順に照会。送信metadataへ全entryを変換し、10人以上は拒否 |
| test::mutated / deliverAction | 既存Outbox・履歴・30秒送信期限・unknown契約を共有。結果不明は自動再実行せず、API成功にも実機確認の案内を付ける |

独立したWorker・Queue・HTTP Client・常時処理は追加しない。Inspectは対象ごとにgetSquareMemberを照会するが、新しい並列枠は作らない。既存管理操作の対象人数は変更しない。

NativeとAdapterをProtocol v18として同時に更新する。以前の単独mention・参加退出通知・mute等はadditional=Noneで同じmetadataを送る。旧Planのseparateはdefault=falseで復元する。

## 検証

OC Smokeへ、未許可者の拒否、previewで投稿しないこと、同範囲の複数MID、重複MIDの排除、絵文字・改行を含むUTF-16範囲、separateの範囲、照会先と実行先の分離、保存後の再起動、単独名変更、9人・10人境界、長すぎる名前・空名の拒否を追加した。既存のAPI mockを使い、実LINEへの通知は送っていない。

実機では同じ2人へ共有範囲と--separateをそれぞれ一度実行し、両者の端末で表示・通知を比較する。共有範囲が一人分に縮約される等の結果が出た場合は、その結果を記録して後続のロール設計を判断する。

初回追加時（コミット382ad95）、build / TypeScript check / Clippy（全workspace・all-targets・警告拒否）、OC・Command・文面Smokeを通過。文面key/schemaの整合性も検査した。同一表示範囲で全員へ端末通知できることは確認できていない。

## 履歴からの復元（2026-10-05）

コミット382ad95の試験を復元し、現在のProtocol v18へ統合した。後から追加したストア・スケジュール通知を維持する。通常のtest mentionは変更しない。廃止時にCommandRetiredで失敗済みとなったActionは復活させず、新規コマンドを受け付ける。保存した未送信試験は共通期限とOutboxに従い、unknownは引き続き明示的な照合が必要。削除時の起動取消と結果抑止は解除した。

APIの送信成功だけでは、同一表示範囲で複数人へ通知できたと判断しない。今回も実機での通知は未検証。

復元後にbuild・TypeScript check・Clippy（workspace / all-targets / release / 警告拒否）とOC・Command・文面・SKD・Persistence・受信配送Smokeを通過した。OC Smokeで同範囲・個別範囲、照会先と投稿先、管理者制限、再起動後の配送、人数と表示名の境界を確認した。文面key/schema・BOM・資料リンク・差分の空白も検査した。
