# 最小Botの起動と次の実験

作成日: 2026-10-02、更新日: 2026-10-03。状態: Native build・型生成・TS build・オフラインSmoke・公開素材による動画生成済み。既存NorthflankへProtocol v6を配備し、PUSH受信・GitHubへの暗号化退避・変換済みログへの定期追記を確認。実Command配送と多数OCの長期負荷は運用観測を続ける。

## 起動

Node 24以降、Rust 1.98、SQLite Cコードをコンパイルできる環境を使う。Windows GNU LLVMではLLVM MinGWのbinをPATHまたは`LLVM_MINGW_BIN`へ指定。今回のローカル検証では`.tools/llvm-mingw-20260922-ucrt-x86_64/bin`を利用した。LinuxはC compilerが必要。依存lockと生成済みProtocolはGitへ保存する。

```sh
npm ci --ignore-scripts
npm run build
npm run smoke
npm run smoke:commands
```

`npm run build`はProtocol型生成、Native release build、TypeScript buildの順。`scripts/native.cjs`はWindowsのNode dynamic symbol対応と必要なlibunwindのコピーも行う。DockerfileはLinuxで同じNativeとTSをbuildし、実行時はnodeユーザーを使う。ローカルDocker Engineは未利用。Northflankで旧版091d981のLinuxイメージbuild成功を確認した。

旧Botと受信Probeを停止してから、旧認証ストレージを機密ファイルとして`storage/auth.json`へコピーする。同じ`LINE_DEVICE`を使い、`.env.example`を`.env`へコピーして`LINE_OLD_BOT_STOPPED=1`にする。保存済み`.auth`があればLINE_AUTH_TOKENは不要。新規QR・passwordログインは実装していない。

```sh
npm start
```

コマンドは`!ping`→`pong!`、`!test-notify 5`→受付案内と5秒後の確認通知。確認通知が来るまで次のコマンドを送らず、自律的な起床を観測する。テスト通知は1〜60秒に限定した確認用機能で、旧`push`通知の移植ではない。

`!help`と`!ut / !tut / !st`も実装済み。[Command仕様](../../crates/kbc-core/src/commands/docs/COMMANDS.md)を参照。Dockerには`content/`と`data/search/`を同梱する。txtと検索データの変更は再起動・再配備で反映する。

`motion`のMP4/GIFにはFFmpegが必要。DockerfileはFFmpegを同梱し、Linuxの既定pathは`/usr/bin/ffmpeg`。Windowsでは`FFMPEG_PATH`へ実行ファイルの絶対pathを指定する。PNGはFFmpegなしでも生成できる。公開素材だけを使う`npm run verify:media`はLINE認証・通信を行わず、PNG/MP4/GIF・file選択と生成中のpingを検査する。[実験結果](../../experiments/commands/docs/MEDIA_VERIFICATION.md)を参照。

## 保存とコンテナの扱い

`storage/auth.json`と`storage/core.sqlite`は同じ運用単位で保管し、公開Gitへ置かない。旧認証のreqseq・refresh情報を保持する。Coreファイルを別アカウントへ使うとOwnerMismatchで停止する。結果不明はSQLiteのactionsに残し、勝手に再投稿しない。

Protocol v6のNativeとAdapterを同時に更新する。重複IDの保持上限は`CORE_MAX_RETAINED_EVENTS=131072`が既定で、8,192〜524,288へ設定できる。SQLiteの64MiB page上限は別に効く。毎分metricsのretainedEvents / maxRetainedEvents、queued / preparingMedia / claimed / querying / sending / unknown / completedActions / activeSessionsと、receiverのpendingChats / chatFailuresを観測する。

Mediaの成果はCore DBの隣の`media/`に置く。未解決8件まで、成果は合計最大64MiBで、生成中の素材・FFmpeg一時ファイル分も必要。DBを保管・復元する場合は未解決成果も同じ保存単位にする。プロセス再起動では生成途中を再準備し、送信途中はunknownとして保持する。成果消失は配送直前に再実行案内へ変えるため、unknownを解決する際に保存ファイルだけで送信成否を判断しない。[Mediaの保存・復旧](../../crates/kbc-core/docs/MEDIA.md)を参照。

保存障害は`AuthStorageError`で全体停止する。ディスク容量・書込先・権限を修復してから同じ保存ファイルで再起動し、未知の認証状態のまま継続させない。容量不足では新規Batchがrollbackされる。結果不明を削除して空きを作らず、実送信結果を照合できたものだけ、ローカルCoreの`resolveAction({ actionId, status: "sent" または "failed", code })`で明示的に解決する。公開healthにはID一覧を出さない。この操作用のチャットCommand / CLIはまだ実装していない。

Northflankは永続Volumeなし。追加料金を使わず、既存の非公開GitHubへ認証・Core状態を暗号化退避し、コンテナ交換で自動復元する。未退避期間の損失と送信成否の照合が必要。詳細は [GitHubへの退避・復元](GITHUB_RECOVERY.md)。

長期ログは旧履歴を軽量形式へ変換して引き継ぎ、OC MID / トークMIDの階層に整理する。認証・設定・Runtimeはログとは別の保存単位。[OCログ方針](../decisions/OC_LOG_STORAGE_V2.md)。

## 次の少数OC実験

1. 旧認証の機密ファイルを新Adapterへ渡す経路、Container消失時の試験データの扱い、切戻しを確定する。旧受信器は動かさない。
2. PUSHのsign-on、イベント種別、API回数、無通信CPU/RSSを記録する。
3. 通常の`!ping`返信と、次の入力なしの`!test-notify 5`を確認する。
4. 多OCとコマンド量は段階的に増やす。重なった入力の返信抜けを再現する手動試験は利用者の指定どおり運用観測へ回す。
5. `origin`・`file`・短い`motion`の実LINE投稿、番号リプライ、管理者削除を確認する。0.2コア・512MiBで生成中のRSS・応答時間・CPUを計測してからFrame範囲を増やす。

オフラインSmokeは一つのファイルで、異なるID、重複、Batch rollback、claimed / sendingからの再開、結果不明、自律期限、アカウント所有者、API枠内のtoken更新、実SDKのreqseq保存障害と送信境界、完了・未解決容量、Thrift sign-on、継続中のPUSH集約、chat補完の部分障害と期限再試行を確認した。[修正の判断と容量Probe](../decisions/FOUNDATION_RECOVERY_V1.md)を参照。実LINEの全件配送・API制限回避・PUSH再接続の成功証明としては扱わない。

## OC管理の起動設定

通常の!oc管理を追加した。BOT mod/adminが必要な操作にはBOT_PERMISSIONS_PATHへ機密permissions.jsonの絶対pathまたはworkspace相対pathを指定する。未指定ではBOT権限なしで、OC ADMIN / CO_ADMINが許可された設定操作だけを使える。初期値の自動処理はOFF。旧認証・権限ファイル・Core DBを公開Gitへcommitしない。

既存Core DBで起動するとOC用tableを作る。設定・mute・参加状態・審議・OC対話は同じDBの退避単位に含める。旧ログの変換・整理とこの状態の継承を混同しない。照会のqueryingは再起動時再待機、更新・通報・削除のsendingはunknownとなる。例外時は実OCと履歴を照合する。

追加のnpm run smoke:ocはLINEへ接続しない。少数OCではBotの管理者削除・membership API権限、入退室の実PUSH種別、同時入力、API回数とquota下のCPU/RSSを測定する。GitHub保存復元はオフライン検証済み。本番配備結果は後続記録。[OCの操作と制限](../../crates/kbc-core/src/oc/docs/OC.md)。

GitHub指定時はBOT_PERMISSIONS_PATH / LEGACY_OC_SETTINGS_PATHの未指定pathをstorage/permissions.json / storage/legacy-oc-settings.jsonとして旧データから配置する。旧OC設定3件・通知設定5トークの取り込みを実データで確認済み。`npm run smoke:persistence` は外部通信なしの復元検証。

## 2026-10-03の本環境起動

75a5dd7 / civil-noise-5798を既存サービスへ配備した。既存0.2vCPU・512MB、1 instance、永続Volume追加なし、CDはOFF。旧認証からlogin、PUSH sign-on、health 200 / receiving、Coreの暗号化GitHub退避成功を確認。最初の観測でAPI requests 4 / errors 0 / rateLimits 0、RSS約119MiB。Northflank表示のCPU <1% / memory 140.51MB・restart 0。長期負荷と実Command配送の検証完了とは扱わない。

新形式ログは `OC_LOGS_ENABLED=1` で有効化する。まず非公開データrepoの変換Workflowを実行し、完了後に新版を配備する。配備は旧instanceを0にして停止を確認してから切り替え、同じアカウントの新旧プロセスを並行稼働させない。Core snapshotが復元した期限済みの副作用は照合待ち。`npm run smoke:logs`は追記・容量切替・manifest失敗後の再開・競合・改名・複数kick・分割ページのcheckpointを外部通信なしで確認する。

## 2026-10-03の軽量ログ切替

旧ログ変換Workflowの反映成功後、a1730c6 / equal-bell-3355を配備した。instanceを一旦0にし、停止後に配備・1へ戻した。旧Core snapshotから起動し、OC_LOGS_ENABLED=1でPUSHを受信した。永続Volume追加なし、0.2vCPU・512MB、CD OFFを維持する。

最初の5分周期でログ8行を同期し、変換済みの既存payload2ファイルに追記した。新payloadは0、manifest2件の件数・gzip全行・SHA256を遠隔blobと照合した。health 200 / receiving、pendingLogs 0、logs cycles 1 / failures 0 / rows 8。起動約349秒時点のAPI requests 10 / errors 0 / rateLimits 0、Core backup 4 / failures 0、RSS約158MiB。短時間観測であり、API制限の回避や大量配送の保証ではない。

2周期後もlogs failures 0。遠隔Git履歴とblobを再照合し、変換済みpayload4ファイルへ累計10行を追加、新payloadは0を確認した。

復元した補完待ち1トークはTypeErrorで再試行待ちのため、subscriptionを必須にする参照を修正した。[調査と判断](../research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md#13-本環境のトーク補完と購読情報)。受信cursorや待機トークを消さず、既存の再試行期限を引き継ぐ。

86c1ca8 / equable-house-3257のbuild成功後、同じ停止・配備・起動手順で反映した。Core復元後の起動約48秒でhealth 200 / receiving、補完待ち0 / 補完失敗0。fetchSquareChatEvents 3回で履歴を含む185イベントを取得し、受付20・重複22・対象外143。API errors 0 / rateLimits 0、RSS約115MiB、Core backup成功。maxLagMsは取得した過去履歴の経過時間を含み、新着の応答時間として解釈しない。新着Command返信は別途実OCで確認する。
