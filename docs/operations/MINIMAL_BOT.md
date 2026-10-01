# 最小Botの起動と次の実験

作成日: 2026-10-02。状態: ローカルのNative build・型生成・TS build・オフラインSmoke済み。新BotのNorthflank配備はまだ行っていない。

## 起動

Node 24以降、Rust 1.98、SQLite Cコードをコンパイルできる環境を使う。Windows GNU LLVMではLLVM MinGWのbinをPATHまたは`LLVM_MINGW_BIN`へ指定。今回のローカル検証では`.tools/llvm-mingw-20260922-ucrt-x86_64/bin`を利用した。LinuxはC compilerが必要。依存lockと生成済みProtocolはGitへ保存する。

```sh
npm ci --ignore-scripts
npm run build
npm run smoke
```

`npm run build`はProtocol型生成、Native release build、TypeScript buildの順。`scripts/native.cjs`はWindowsのNode dynamic symbol対応と必要なlibunwindのコピーも行う。DockerfileはLinuxで同じNativeとTSをbuildし、実行時はnodeユーザーを使う。Dockerイメージ自体は今回のローカル環境で未検証。

旧Botと受信Probeを停止してから、旧認証ストレージを機密ファイルとして`storage/auth.json`へコピーする。同じ`LINE_DEVICE`を使い、`.env.example`を`.env`へコピーして`LINE_OLD_BOT_STOPPED=1`にする。保存済み`.auth`があればLINE_AUTH_TOKENは不要。新規QR・passwordログインは実装していない。

```sh
npm start
```

コマンドは`o.ping`→`pong!`、`o.test-notify 5`→受付案内と5秒後の確認通知。確認通知が来るまで次のコマンドを送らず、自律的な起床を観測する。テスト通知は1〜60秒に限定した確認用機能で、旧`push`通知の移植ではない。

## 保存とコンテナの扱い

`storage/auth.json`と`storage/core.sqlite`は同じ運用単位で保管し、公開Gitへ置かない。旧認証のreqseq・refresh情報を保持する。Coreファイルを別アカウントへ使うとOwnerMismatchで停止する。結果不明はSQLiteのactionsに残し、勝手に再投稿しない。

現在のNorthflankサービスは永続Volumeなし。SQLiteは同じファイルが残るプロセス再起動では復元できるが、コンテナ交換・再配備で失われる。予定通知・受付記録の永続性を本運用で主張する前に、永続ディスクまたは整合した退避・復元方式を確定する。旧Botの暗号化GitHubバックアップからの自動復元・新しいCoreの退避は今回未実装。Dockerfileだけで現行コンテナへ切り替えると認証ファイルを引き継げない。

退避先は旧Botと同じ非公開GitHubストレージを使う方針。長期OCログは旧履歴を読み込まず、OC MID / トークMIDの階層で新規開始する。長期ログの新規開始を理由に認証・reqseq・RuntimeのcheckpointやActionを削除しない。現時点では長期ログの保存・同期も未実装。[新しいOCログの保存方針](../decisions/OC_LOG_STORAGE_V2.md)に切り替えと実装順を記録する。

## 次の少数OC実験

1. 旧認証の機密ファイルを新Adapterへ渡す経路、Container消失時の試験データの扱い、切戻しを確定する。旧受信器は動かさない。
2. PUSHのsign-on、イベント種別、API回数、無通信CPU/RSSを記録する。
3. 通常の`o.ping`返信と、次の入力なしの`o.test-notify 5`を確認する。
4. 多OCとコマンド量は段階的に増やす。重なった入力の返信抜けを再現する手動試験は利用者の指定どおり運用観測へ回す。

オフラインSmokeは一つのファイルで、異なるID、重複、Batch rollback、保存からの再開、結果不明、自律期限、アカウント所有者、API枠内のtoken更新、実SDKのThrift sign-on、継続中のPUSH集約、chatのページ補完を確認した。実LINEの全件配送・API制限回避・PUSH再接続の成功証明としては扱わない。
