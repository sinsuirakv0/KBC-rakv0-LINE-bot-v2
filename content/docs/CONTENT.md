# txtからの応答とhelp

状態: 実装済み。読み込みはRustの `commands/content.rs`。

`responses/<name>.txt` を追加すると `!<name>` が使える。`help/<name>.txt` は `!help <name>` の案内になる。応答txtがなくてもhelp txtだけで案内を追加できる。helpがなければ応答本文を案内として使う。`help/index.txt` は全体案内の冒頭で、実装済みコマンドの一覧はCoreが追加する。

日本語を含むtxtはBOM付きUTF-8、表示はプレーンテキスト。コードブロックと空ファイルは起動時に拒否する。1ファイルは8KiB・1,500 UTF-16単位まで、各フォルダ128ファイルまで。ファイル名は英小文字・数字・`_`・`-` の32文字以内。`help`、`ut`、`tut`、`st`、その別名、`test-notify`、`test`、`oc`、`oc-admin`、`id`、`bot` は処理を持つためresponsesの名前に使えない。

起動時に一度読み込む。追加・変更は再起動または再配備で反映する。コマンド名とhelpの対象名はASCIIの大文字小文字を区別しない。静的応答の追加引数はDiscordと同様に無視する。`!コマンド help`は案内への切替として使わない。

検索・番号リプライの詳細は [Command実装](../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

OCの公開案内はhelp/oc.txt。help/oc-admin.txtはinternal_helpから権限確認後だけ返し、!help oc-adminでは公開しない。OC管理の実処理は [専用実装](../../crates/kbc-core/src/oc/docs/OC.md) に置く。

help/id.txtも起動時に検証する。!idの通常案内はcatalog、取得・検索は [OCの共通照会](../../crates/kbc-core/src/oc/docs/ID.md) に渡す。

検索のhelpと一覧は1ページ10件とし、一覧への1〜10のリプライで項目を選ぶ。ページ移動は「次」「前」、ページ指定は「3p」のようにリプライする。リアクション操作と案内の標準LINE絵文字は廃止した。help/id.txtのsticker / emojiによる受信済みID参照は維持する。

help/test.txtは!test / !help testの共通案内。!test replyは [送信テスト](../../crates/kbc-core/src/oc/docs/TEST_REPLY.md)、allowの複数OC登録とメンション・管理操作は [検証OCの実装](../../crates/kbc-core/src/oc/docs/TEST_OC.md) でBOT管理者の権限を確認する。

同じhelpに[!test sticker](../../crates/kbc-core/src/oc/docs/TEST_STICKER.md)のセットID・スタンプID・任意version / STKOPTを案内する。実行トークへの1回送信で、allow登録・--applyを要求しない。

!test mentionの--target-chatはメンバーの所属照会先と案内する。実行トークでメンションし、他の管理操作の対象トーク指定と区別する。

2026-10-04、helpの入口を!help <name>へ統一した。Command Smokeでhelp txtだけの追加、大文字の対象名、静的応答のhelp引数が応答本文になること、旧!bot helpを案内として扱わないことを確認した。通常の管理・リプライ・検索・IDのOC Smoke、build、型検査、Clippyも通過。公開helpと、権限確認後だけ返すOC内部案内の区別は維持する。

help/bot.txtは!bot / !help botの共通案内。botは必須help・コマンド一覧へ登録し、[表示名変更](../../crates/kbc-core/src/oc/docs/BOT.md) の実処理はRust OC基盤で行う。
