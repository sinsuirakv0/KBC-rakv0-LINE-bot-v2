# txtからの応答とhelp

状態: 実装済み。読み込みはRustの `commands/content.rs`。

`responses/<name>.txt` を追加すると `!<name>` が使える。対応する `help/<name>.txt` は `!<name> help` / `!help <name>` の案内になる。helpがなければ応答本文を案内として使う。`help/index.txt` は全体案内の冒頭で、実装済みコマンドの一覧はCoreが追加する。

日本語を含むtxtはBOM付きUTF-8、表示はプレーンテキスト。コードブロックと空ファイルは起動時に拒否する。1ファイルは8KiB・1,500 UTF-16単位まで、各フォルダ128ファイルまで。ファイル名は英小文字・数字・`_`・`-` の32文字以内。`help`、`ut`、`tut`、`st`、その別名、`test-notify`、`test`、`oc`、`oc-admin`、`id` は処理を持つためresponsesの名前に使えない。

起動時に一度読み込む。追加・変更は再起動または再配備で反映する。コマンド名はASCIIの大文字小文字を区別しない。静的応答の追加引数はDiscordと同様に無視し、最初の引数が `help` の場合だけ案内を返す。

検索・番号リプライの詳細は [Command実装](../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

OCの公開案内はhelp/oc.txt。help/oc-admin.txtはinternal_helpから権限確認後だけ返し、!help oc-adminでは公開しない。OC管理の実処理は [専用実装](../../crates/kbc-core/src/oc/docs/OC.md) に置く。

help/id.txtも起動時に検証する。!idの通常案内はcatalog、取得・検索は [OCの共通照会](../../crates/kbc-core/src/oc/docs/ID.md) に渡す。

help/test.txtは!test / !test help / !test reply help / !help testの共通案内。!test replyだけを実装し、BOT管理者の権限判定は [OCの共通照会と送信テスト](../../crates/kbc-core/src/oc/docs/TEST_REPLY.md) で行う。
