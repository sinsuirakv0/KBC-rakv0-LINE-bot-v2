# 指定IDによるスタンプ送信テスト

2026-10-04。実装済み。LINEJS 3.4.2（npm revision 11）のSquareService.sendMessageとStickerMetadataを確認した。実LINEでの受理・表示・所有条件は未確認。少数のスタンプでAPI応答と端末表示を観測する。

## 入力と権限

`!test sticker <セットID> <スタンプID>` で実行トークへ1件送信する。`o.test`も同じ。BOT admin（実行OCまたは実行トークのrank 2）だけが使え、共通Contextで実行者のMID・OC・JOINEDを確認する。

`--version <数字>`と`--option <STKOPT>`は順不同で各1回。versionの既定値は1、STKOPTは省略可能。セットID・スタンプID・versionはASCII数字1〜64文字、STKOPTはASCII英数字・アンダースコア1〜64文字に限定する。引数不足・重複・未知の引数ではAPI操作を登録しない。

対象スタンプへのリプライで`!id sticker`を使い、受信したID・version・STKOPTを参照できる。STKTXTは代替文「[スタンプ]」とする。送信先は実行トークに固定し、リプライmetadata・別トーク指定・複数送信を追加しない。allow登録・--applyは不要で、コマンド実行が送信指示になる。

## 関数と配送

| 関数・型 | 働きと関係 |
| --- | --- |
| test::parse / execute | 共通ContextのBOT管理者権限を確認し、stickerへ分岐 |
| test::sticker | 引数検証後、既存Plan / JobをMutationとしてOutboxへ登録 |
| OcRequest::Sticker | Protocol v13。package_id・sticker_id・version・optionだけを渡すplain DTO |
| deliverAction / ApiScheduler | 既存配送WorkerとAPI枠を共有。sendMessageの実fetch直前にsendingを保存 |
| SquareDirectory.execute | contentType=STICKERとSTK metadataをSDKへ渡し、返されたmessage IDを確認 |
| test::mutated | 共通ActionResultからID・API code・送信message IDを通知し、oc_historyへ保存 |

NativeとAdapterを同時更新する。旧保存DTOの形は変えず、新たなQueue・Worker・画像ダウンロード・巡回を作らない。

既存テスト操作と同じく、受信からContext照会まで60秒、Mutation登録から通信開始まで30秒を期限とする。通信前失敗だけ再待機する。通信後の例外や返値のmessage ID欠落はunknownで、自動再送しない。sending中の再起動もunknownとなる。resolveActionは保存履歴だけを更新し、送信と結果通知を再実行しない。

API受理と端末表示は別に確認する。IDだけで所有・公開状態を問わず送れるとは判断しない。公式Messaging APIの送信可能スタンプ一覧をSquareへ適用しない。[SDK調査](../../../../../apps/line/docs/REACTIONS_AND_STICKERS.md)。

## 検証の範囲

既存smoke:ocのSDK mockで、権限拒否・不正入力・既定version・明示version / STKOPT・実行サブトーク・リプライなし・Mutation登録後の再起動・通信後拒否・message ID欠落・unknownの再送抑止を確認した。模擬SDKの成功を実LINE表示の成功とは扱わない。

Windows / Node 24.15.0 / Rust GNU LLVMでbuild、型検査、smoke / smoke:commands / smoke:oc、Clippy（workspace・all-targets・release・locked・警告をエラー扱い）が通過した。共通helpは1,273 UTF-16単位で1,500単位以内。BOM・LF・資料リンク・git diff --checkも確認した。
