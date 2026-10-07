# txtからの応答とhelp

状態: 実装済み。読み込みはRustの `commands/content.rs`。

返信・一覧・通知・利用者向けエラーの共通文面は`messages/<用途>.txt`の`key = 本文`で編集する。権限案内などはcommon.txtを共有する。[形式・差し込み項目・追加方法](../../crates/kbc-core/docs/MESSAGES.md)。定型応答とhelp本文は以下のtxtを使う。

`responses/<name>.txt` を追加すると `!<name>` が使える。`help/<name>.txt` は `!help <name>` の案内になる。応答txtがなくてもhelp txtだけで案内を追加できる。helpがなければ応答本文を案内として使う。`!help` / `o.help`は`help/index.txt`の本文だけを表示する。コマンド一覧はこのファイルへ手動で記載し、txt追加による自動追加は行わない。

日本語を含むtxtはBOM付きUTF-8、表示はプレーンテキスト。コードブロックと空ファイルは起動時に拒否する。1ファイルは8KiB・1,500 UTF-16単位まで、各フォルダ128ファイルまで。ファイル名は英小文字・数字・`_`・`-` の32文字以内。`skd`、`pushsetting`、`help`、`ut`、`tut`、`st`、その別名、`test-notify`、`test`、`oc`、`oc-admin`、`id`、`bot` は処理を持つためresponsesの名前に使えない。

起動時に一度読み込む。追加・変更は再起動または再配備で反映する。コマンド名とhelpの対象名はASCIIの大文字小文字を区別しない。静的応答の追加引数はDiscordと同様に無視する。`!コマンド help`は案内への切替として使わない。

検索・番号リプライの詳細は [Command実装](../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

OCの公開案内はhelp/oc.txt。help/oc-admin.txtはinternal_helpから権限確認後だけ返し、!help oc-adminでは公開しない。OC管理の実処理は [専用実装](../../crates/kbc-core/src/oc/docs/OC.md) に置く。

help/id.txtも起動時に検証する。!idの通常案内はcatalog、取得・検索は [OCの共通照会](../../crates/kbc-core/src/oc/docs/ID.md) に渡す。

検索のhelpと一覧は1ページ10件とし、一覧への1〜10のリプライで項目を選ぶ。ページ移動は「次」「前」、ページ指定は「3p」のようにリプライする。リアクション操作と案内の標準LINE絵文字は廃止した。help/id.txtのsticker / emojiによる受信済みID参照は維持する。

help/test.txtは!test / !help testの共通案内。!test replyは [送信テスト](../../crates/kbc-core/src/oc/docs/TEST_REPLY.md)、allowの複数OC登録とメンション・管理操作は [検証OCの実装](../../crates/kbc-core/src/oc/docs/TEST_OC.md) でBOT管理者の権限を確認する。

同じhelpに[!test sticker](../../crates/kbc-core/src/oc/docs/TEST_STICKER.md)のセットID・スタンプID・任意version / STKOPTを案内する。実行トークへの1回送信で、allow登録・--applyを要求しない。

!test mentionの--target-chatはメンバーの所属照会先と案内する。実行トークでメンションし、他の管理操作の対象トーク指定と区別する。

2026-10-04、helpの入口を!help <name>へ統一した。Command Smokeでhelp txtだけの追加、大文字の対象名、静的応答のhelp引数が応答本文になること、旧!bot helpを案内として扱わないことを確認した。通常の管理・リプライ・検索・IDのOC Smoke、build、型検査、Clippyも通過。公開helpと、権限確認後だけ返すOC内部案内の区別は維持する。

help/bot.txtは!bot / !help botの共通案内。botは必須helpへ登録し、[表示名変更](../../crates/kbc-core/src/oc/docs/BOT.md) の実処理はRust OC基盤で行う。

2026-10-04、全体helpをindex.txtの本文だけに変更した。Command Smokeでresponsesと個別helpを追加した状態の!help / o.helpが手動のindex本文と一致し、個別helpと応答の追加は引き続き使えることを確認。buildと型検査も通過した。

pushsettingは処理を持つ予約名。help/pushsetting.txtは公開案内、messages/update.txtは登録応答・状態表示・ストア更新通知の全定型文面。!helpのindexへは手動で項目を追加した。[仕様](../../crates/kbc-core/src/store_update/docs/STORE_UPDATE.md)。

!skdはresponsesの予約名。help/skd.txtは表示・日付指定、help/pushsetting.txtはskd通知登録を案内する。利用者向けスレッド案内・差分ラベルはmessages/skd.txtへ分離し、indexは手動追記した。[スケジュール仕様](../../crates/kbc-core/src/skd/docs/SKD.md)。

help/test-mention.txtは!test mention-labelの専用案内。入口のhelp/test.txtへ手動で参照を追加した。名前変更・複数人の共有範囲・--separateの比較を説明し、API成功と実際の通知を区別する。[仕様](../../crates/kbc-core/src/oc/docs/TEST_MENTION.md)。

2026-10-06、help/bot.txtへBOT権限の登録・解除・一覧、停止/再開、statusを追加した。messages/bot.txtとschemaのキーで全定型文面を編集できる。indexには既存bot項目を使い、自動で追記しない。[管理機能](../../crates/kbc-core/src/oc/docs/BOT.md)。

2026-10-07、gatya・sale・itemを処理を持つ予約名と必須helpへ追加した。help/indexへ3コマンドを手動追記。表示はmessages/event.txtと既存skd等の共通キーで変更できる。saleのリプライ番号選択を公開helpへ記載した。[実装](../../crates/kbc-core/src/event_data/docs/COMMANDS.md)。
