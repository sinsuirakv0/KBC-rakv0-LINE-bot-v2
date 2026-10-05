# 最小Botの起動と次の実験

作成日: 2026-10-02、更新日: 2026-10-04。状態: Native build・型生成・TS build・オフラインSmoke・公開素材による動画生成済み。既存NorthflankへProtocol v7を配備し、PUSH・独立したトーク取得・GitHubへの暗号化退避・軽量ログへの追記を確認。実Commandごとの表示と多数OCの長期負荷は運用観測を続ける。

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

`!help`と`!ut / !tut / !st`も実装済み。[Command仕様](../../crates/kbc-core/src/commands/docs/COMMANDS.md)を参照。Dockerには`content/`と`data/search/`を同梱する。txtの変更は再起動・再配備で反映する。検索データは公開mainを90秒周期で自動確認し、2分で失効する。SEARCH_DATA_PATHは更新用の書込可能path（既定storage/search/catalog.json）を指定する。初回取得中や公開データ障害でも受信・pingは継続する。[更新の運用](../../scripts/docs/SEARCH_DATA.md)。

`motion`のMP4/GIFにはFFmpegが必要。DockerfileはFFmpegを同梱し、Linuxの既定pathは`/usr/bin/ffmpeg`。Windowsでは`FFMPEG_PATH`へ実行ファイルの絶対pathを指定する。PNGはFFmpegなしでも生成できる。公開素材だけを使う`npm run verify:media`はLINE認証・通信を行わず、PNG/MP4/GIF・file選択と生成中のpingを検査する。[実験結果](../../experiments/commands/docs/MEDIA_VERIFICATION.md)を参照。

## 保存とコンテナの扱い

`storage/auth.json`と`storage/core.sqlite`は同じ運用単位で保管し、公開Gitへ置かない。旧認証のreqseq・refresh情報を保持する。Coreファイルを別アカウントへ使うとOwnerMismatchで停止する。結果不明はSQLiteのactionsに残し、勝手に再投稿しない。

Protocol v8のNativeとAdapterを同時に更新する。重複IDの保持上限は`CORE_MAX_RETAINED_EVENTS=131072`が既定で、8,192〜524,288へ設定できる。SQLiteの64MiB page上限は別に効く。毎分metricsのretainedEvents / maxRetainedEvents、queued / preparingMedia / claimed / querying / sending / unknown / completedActions / activeSessionsと、receiverのpendingChats / chatFailuresを観測する。

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

## 2026-10-04の通常応答・ID・参加取得・Motion修正

6825fa2 / brief-test-8821へ、Protocol v7のNativeとAdapterを同時配備した。既存instanceを0へ変更して停止を確認し、配備後に1へ戻した。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持した。

通常送信を既定にし、原因投稿を示す副官通知は同じOCのサブトークreplyを使う。!idの自分・対象・名前検索・message/reply参照を追加。log allは指定どおり後回し。[IDの範囲と上限](../../crates/kbc-core/src/oc/docs/ID.md)。旧OC管理との表示・権限・mute警告・再参加条件の差は[旧仕様照合](../research/OC_LEGACY_COMPATIBILITY_2026_10_03.md)へ記録した。SendBoundaryNotReachedの再起動経路はApiSchedulerのSendAttempt Context維持で修正した。

約654秒の観測でhealth 200 / receiving、PUSH session 1、定期取得12トーク・優先5トーク、829周期。API requests 907 / errors 9 / rateLimits 0、Action完了4件（sendMessage RPC 3回を含む）・新しいAction失敗/unknown 0。旧unknown 8件は保持しており、成功と推定して消したり再送したりしない。CPUのNorthflank表示は0.0073vCPU（割当の約4%）、Memory 167.84MB、restart 0。RSSは約153MiB。Motion生成中や長期負荷の値ではない。

暗号化backup 11回 / failures 0、ログ同期2周期 / failures 0。同期27行はmessages 21 / names 3 / member-events 3、既存payload8件への追記と新payload1件。遠隔manifest9件のSHA256・件数・gzip全行を照合した。参加退出のsource=poll・元eventType・receivedAtMsも保存されていた。[実測と未参加候補](../research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md#15-参加トーク一覧の実api差2026-10-04)。

全5種のSmoke、buildとClippyを確認した。Motionは同じDockerfileの0.2CPU / 512MiBでMP4/GIF/PNGとメモリ圧迫時の生成中止・後続pingを検証し、OOMKilled=false。[測定条件と限界](../../experiments/motion-memory/docs/MEMORY.md)。報告されたメモリ異常そのものは同じ公開素材で再現できておらず、対策後の実Bot生成負荷は観測を続ける。

続いて8a14f53 / natural-hands-3186へ切り替えた。参加一覧にない旧通知設定1トークを定期取得から除外し、設定は保持した。listedChats=11 / pollingChats=11 / priorityChats=4 / unlistedPriorityChats=1。起動約146秒で定期取得181周期、API requests 184 / errors 0 / rateLimits 0、PUSH session 1、暗号化backup 3回 / failures 0、RSS約127MiB。約3分時点のNorthflank表示はCPU 0.011vCPU（割当の約6%）、Memory 141.50MB、restart 0。新規投稿のない観測期間であり、Command応答時間の測定とは扱わない。前版のNOT_FOUNDはこの期間には出なかった。

## 2026-10-04のリプライ送信テスト配備

a42937f / boiling-throne-4097へ、BOT管理者限定の!test replyを配備した。Linux buildのSuccessを確認し、旧instanceを0へ変更して0 / 0を確認してから配備・1へ戻した。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持。Protocolは既存v7のまま。[コマンド・関数・検証範囲](../../crates/kbc-core/src/oc/docs/TEST_REPLY.md)。

起動約135秒でhealth 200 / receiving、PUSH session / sign-on各1、listedChats / pollingChats各11、priorityChats 4、補完・定期取得の失敗0。API requests 171 / errors 0 / rateLimits 0、暗号化backup 3回 / failures 0、RSS約129MiB。Coreの完了82件・旧unknown 8件を復元し、新規配送の失敗・unknownは0。新起動後のログ12行は5分同期の待機中で、この時点で新周期の完了は未確認。Northflank表示はCPU 0.0067vCPU（割当の約3%）、Memory 142.23MB、restart 0。実LINEでの!test reply投稿・サブトーク間/別OCの表示は利用者の試験対象であり、この起動確認を実送信成功の証明としない。

続く利用者の仕様訂正で、トークMIDは返信元メッセージがあるトークを表し、投稿先はコマンド実行トークと確定した。初版の--toによる送信先変更を廃止し、--chatによる返信元指定・受信済み情報との照合へ修正。3dd8c61 / capable-horn-3678のLinux build成功後、0 / 0の停止確認→配備→1 instanceへの起動で反映した。既存v7・0.2vCPU / 512MB・Volumeなし・CD OFFを維持する。

切替直前の初版はbackup 31回 / failures 0、ログ同期6周期・26行 / failures 0、pendingLogs 0。修正版の起動約114秒でhealth 200 / receiving、PUSH session / sign-on各1、11トークの定期取得、優先4トーク、補完・定期取得の失敗0。API requests 146 / errors 0 / rateLimits 0、backup 2回 / failures 0、RSS約129MiB。完了87件・旧unknown 8件を復元し、新規配送の失敗・unknownは0。ログ2行は5分同期の待機中。Northflank表示はCPU 0.0081vCPU（割当の約4%）、Memory 141.37MB、restart 0。修正版の実LINEリプライ投稿・別OC表示は未確認。

## 2026-10-04の検証OC操作配備

f69a3bc / special-yam-6537のLinux build成功を確認し、旧instanceを0へ変更して0 / 0の停止確認後、新版の配備・1 instanceへの起動を行った。Protocol v8のNativeとAdapterを同時に更新した。既存0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。[複数OC登録・管理操作の入力と上限](../../crates/kbc-core/src/oc/docs/TEST_OC.md)。

切替直前はhealth 200 / receiving、PUSH session 2、Core完了108件・旧unknown 8件、待機・実行中0。GitHub退避356回・最後の退避成功、累積failures 4。ログ同期72周期・70行・累積failures 1、pendingLogs 0。これらの累積失敗を新版で発生した値として扱わない。

新版の起動約160秒でhealth 200 / receiving、PUSH session / sign-on各1、11トークの定期取得・優先4、補完・定期取得・一覧取得の失敗0。API requests 199 / errors 0 / rateLimits 0、GitHub退避3回 / failures 0、RSS約128MiB。既存Core完了108件・unknown 8件を復元し、新規配送の失敗・unknownは0。NorthflankはRunning / 1 / 1・restart 0、CPU 0.0091vCPU（割当の約5%）、Memory 138.88MB。

新しい検証OC許可は未登録で開始し、!test allow <sMID> <sMID> ... でBOT管理者が管理下のOCだけを登録する。確認表示だけでは変更せず、--apply付きで対象側の実APIを1回試す。Codexからメンション・削除・退会・役割変更の実LINEテスト操作は行っていない。実際の権限不足・別OC通知・役割変更の可否は後続の利用者試験で観測する。5種類のSmoke・build・Clippyはオフラインで通過済み。

## 2026-10-04の検証OC登録・MID案内修正

利用者の実ログで対象OCだけがallow登録済み、実行OCは未登録と確認した。また、--target-chatのsMID誤入力が登録判定に隠れていた。引数形式を先に確認し、未登録側のsMIDと登録コマンドを表示する修正を8703967 / graceful-bear-2760へ配備した。Linux buildは2分7秒で成功。旧instanceの0 / 0を確認してから配備・1 instanceへ戻し、v8・0.2vCPU / 512MB・Volumeなし・CD OFFを維持した。[原因と入力の使い分け](../../crates/kbc-core/src/oc/docs/TEST_OC.md)。

起動約31秒でhealth 200 / receiving、PUSH session / sign-on各1、11トークの定期取得・優先4、Core完了124件・既存unknown 8件・受信保持1,530件を復元した。待機・実行中・新規配送の失敗とunknownは0。API requests 34 / errors 0 / rateLimits 0、GitHub退避1回 / failures 0、RSS約120MiB。この観測期間に新規メッセージはなく、実LINEのadmin実行成功や応答時間の測定とは扱わない。修正のbuild・smoke:oc・smoke:commands・ClippyとBOM / LF / 資料リンク確認は通過。実行OC・対象OCの許可要件やサーバーの権限判定は変更していない。

## 2026-10-04のBot表示名変更配備

b76f157 / next-story-1765のLinux buildは2分6秒で成功。旧instanceの0 / 0を確認してから配備・1 instanceへ戻し、Protocol v9のNativeとAdapterを同時に更新した。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。BOT管理者だけが!bot name 名前（o.も可）を使える。[入力・関数・更新API・検証範囲](../../crates/kbc-core/src/oc/docs/BOT.md)。

切替前はhealth 200 / receiving、Core完了158件・既存unknown 10件、待機Action 1件、照会・送信中0。GitHub退避45回 / failures 0、ログ同期9周期・51行 / failures 0、pendingLogs 0。旧版で発生した不明結果は再実行せず、新版へ引き継いだ。

新版の起動約95秒でhealth 200 / receiving、PUSH session / sign-on各1、11トークの定期取得・優先4、補完・一覧・定期取得の失敗0。API requests 123 / errors 0 / rateLimits 0、GitHub退避2回 / failures 0、Node RSS約128MiB。Core完了158件・unknown 10件、待機Action 1件、照会・送信中0。新起動後の配送失敗・unknownは0。ログ3行は5分同期の待機中で、この時点で新周期の完了は未確認。NorthflankはRunning / 1 / 1・restart 0を確認した。build、3種類のSmoke、Clippy、BOM / LF / 資料リンク検査は通過。Codexから実LINEの名前変更APIは発行していない。配備・起動確認を実LINEでの改名成功として扱わず、利用者が希望の名前で試す。

## 2026-10-04のBot表示名制限解除・再確認配備

名前の独自文字数上限・改行拒否・制御文字拒否を削除し、空の名前だけをUsageにした。`!bot name `の区切り空白を1個だけ除去し、残りの入力は未加工でLINEJSへ渡す。実際の受理可否はLINE側の仕様に委ねる。前版の実LINE観測で`updateSquareMember`の返却が完全なMember DTOではなく、2件を`InvalidSquareMemberResponse`として結果不明にしたため、更新後に同じMemberを1回だけ再取得して状態と表示名を確認する方式へ修正した。結果不明の自動再送は行わない。[入力と再取得確認](../../crates/kbc-core/src/oc/docs/BOT.md)。

bc805ff / worthy-office-3638のLinux build成功後、旧instanceを0へ変更して0 / 0を確認し、新版を配備して1 instanceへ戻した。Protocol v10のNativeとAdapterを同時に更新。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。起動約15秒のhealthは200 / receiving、PUSH session / sign-on各1、API requests 18 / errors 0 / rateLimits 0、待機・照会・送信中0、配送失敗0 / unknown0、GitHub退避1回 / failures 0、RSS約120MiB、restart 0だった。CoreのunknownActions 13は旧状態からの累積値であり、新起動後に増えていない。

build、型検査、smoke:oc、変更検査、git diff --checkは通過済み。今回の切替後は実LINEの名前変更コマンドをまだ発行していないため、任意の名前が実際に受理されることは未確認。利用者が`!bot name 名前`または`o.bot name 名前`で確認する。

## 2026-10-04の検索リアクションページ配備

b1e9dc5 / august-sheep-2077のLinux buildは2分10秒で成功。旧instanceを0へ変更して0 / 0を確認後、新版を配備して1 instanceへ戻した。Protocol v11のNativeとAdapterを同時に更新した。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。

ut / tut / stとファイル一覧の番号リプライを項目選択へ限定し、一覧メッセージの👍（NICE）を次ページ、❤️（LOVE）を前ページにした。リアクション通知に利用者MIDがないため、対象の最新一覧だけ既存OutboxとAPI制御で取得し、検索者を確認する。送信成功まで旧ページを保持し、配送中の選択・通知の連打・旧一覧への操作を区別する。[SDK仕様・関数・検証範囲](../../apps/line/docs/REACTIONS_AND_STICKERS.md)。

build、型検査、smoke / smoke:commands / smoke:oc、Clippy、BOM / LF / 資料リンク検査を通過した。実LINEの通知47の到達とgetMessageReactionsの受理は利用者の操作で確認する。スタンプは受信metadataと送信経路を調査・記録した段階で、送信コマンドは今回追加していない。

15:27 JST、起動約21秒でhealth 200 / receiving、PUSH session / sign-on各1、12トークの定期取得・優先4、補完・定期取得・一覧取得の失敗0。API requests 20 / errors 0 / rateLimits 0、GitHub退避1回 / failures 0、RSS約121MiB。Core完了191件・既存unknown 15件を復元し、待機・照会・送信中0、新規配送の失敗・unknown0を確認した。NorthflankはRunning / 1 / 1、起動約1分でrestart 0を確認。旧unknownは推測で再送しない。新起動後のログ同期周期と実Command配送はこの初期観測では未実施。

## 2026-10-04のLINE標準絵文字・装飾ID配備

d99b74f / hearty-page-3421のLinux buildは2分10秒で成功。旧instanceを0にして0 / 0の停止を確認後、Protocol v12のNativeとAdapterを同時に配備し、1 instanceへ戻した。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。

一覧とHelpのページ案内にLINE標準絵文字の装飾metadataを付ける。操作は一覧メッセージを長押しして付ける実リアクション（NICEで次、LOVEで前）で、数字リプライは項目選択のみ。標準セットのproductIdと143 / 165は公式一覧と実画像を確認したが、Squareでの装飾表示は未確認。[metadata・受信通知・検証範囲](../../apps/line/docs/REACTIONS_AND_STICKERS.md)。

`!id sticker`（`stamp`も可）・`!id emoji`を追加し、新たに受信した投稿から必要なID・version・表示位置だけを保存する。対象へのリプライ、受信済みmessage IDと同じOCの`--chat`指定、コマンド自身のLINE絵文字に対応する。以前の保存参照には装飾IDがないため、配備後に対象を新しく送る。Unicode絵文字にはLINE装飾IDがない。[入力・関数・保持上限](../../crates/kbc-core/src/oc/docs/ID.md)。

16:06 JST、起動約20秒でhealth 200 / receiving、PUSH session / sign-on各1、12トークの定期取得・優先4、補完・定期取得・一覧取得の失敗0。API requests 22 / errors 0 / rateLimits 0、GitHub退避1回 / failures 0、RSS約123MiB。Core完了193件・既存unknown 15件を復元し、待機・照会・送信中0、新規配送の失敗・unknown0を確認した。NorthflankはRunning / 1 / 1、起動約1分でrestart 0を確認。ログ1行は5分同期の待機中。実LINEの装飾表示・新しいIDコマンドの応答・ページ移動成功はこの初期観測では未確認。

build、型検査、smoke / smoke:commands / smoke:oc、Clippyを通過した。装飾のUTF-16位置、mock送信metadata、IDの再起動復元・不正入力・20件上限・別OC参照拒否を既存Smokeで確認した。前版の実受信で通知47を1件観測したが、対象SessionがなくgetMessageReactionsは呼ばれておらず、ページ移動成功の証明とは扱わない。

## 2026-10-04のリプライページ操作への切替

利用者の実運用報告でリアクションによるページ移動を廃止し、最新指定の1ページ10件へ変更した。一覧への「次」「前」、または「3p」のリプライでページを移動し、1〜10は項目選択だけに使う。送信成功まで旧ページを保持する仕組み、スタンプ・LINE絵文字のID取得は維持する。表示件数の違う古い検索Sessionは起動時に失効するため、配備後は検索し直す。[現行仕様・関数・失敗時の扱い](../../crates/kbc-core/src/commands/docs/COMMANDS.md)。

7df606b / tangible-grass-6261のLinux buildは2分19秒で成功。旧instanceを0にして0 / 0の停止を確認後、NativeとAdapterを同時に配備して1 instanceへ戻した。Protocol v12、0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。

切替直前はCore完了199件・既存unknown 15件、待機・照会・送信中・pendingLogs 0。GitHub退避25回 / failures 0、ログ同期4周期・9行 / failures 0。16:34 JST、新版の起動約24秒でhealth 200 / receiving、PUSH session / sign-on各1、12トークの定期取得・優先4、補完・定期取得・一覧取得の失敗0。API requests 23 / errors 0 / rateLimits 0、GitHub退避1回 / failures 0、RSS約122MiB。Core完了199件・既存unknown 15件を復元し、待機・照会・送信中0、新規配送の失敗・unknown0を確認した。NorthflankはRunning / 1 / 1、起動約1分でrestart 0を確認。旧unknownは推測で再送しない。新起動後のログ同期周期と実OCのページ操作はこの初期観測では未確認。

build、型検査、smoke / smoke:commands / smoke:oc、Clippyを通過した。既存Command Smokeで10番選択、次/前/指定ページ、ページ端・0p・overflow、同じページの投稿抑制、切替中の番号とページ操作、確定失敗・再起動復帰、旧8件Session失効を確認した。リアクション通知から追加Actionを作らず、旧Reactions要求の復元時にもAPIを呼ばないことを確認した。実OCでの投稿・管理者削除の可否は利用者の運用観測を続ける。

## 2026-10-04の指定IDスタンプ送信テスト配備

4e0c00b / famous-camp-1232のLinux buildは2分20秒で成功。旧instanceを0にして0 / 0の停止を確認後、Protocol v13のNativeとAdapterを同時に配備し、1 instanceへ戻した。0.2vCPU / 512MB、Volume追加なし、CD OFFを維持する。

BOT管理者専用の!test sticker <セットID> <スタンプID>を追加した。--version / --optionも指定でき、versionの既定値は1。実行トークへ1件送信し、API codeと返された送信message IDを通知する。allow登録・--applyは不要で、通信後unknownは自動再送しない。[入力・関数・検証範囲](../../crates/kbc-core/src/oc/docs/TEST_STICKER.md)。

切替直前はhealth 200 / receiving、Core完了208件・既存unknown 15件、待機・照会・送信中・pendingLogs 0。GitHub退避23回 / failures 0、ログ同期4周期・10行 / failures 0。17:00 JST、新版の起動約18秒でhealth 200 / receiving、PUSH session / sign-on各1、12トークの定期取得・優先4、補完・定期取得・一覧取得の失敗0。API requests 21 / errors 0 / rateLimits 0、GitHub退避1回 / failures 0、RSS約122MiB。Core完了208件・既存unknown 15件を復元し、待機・照会・送信中0、新規配送の失敗・unknown0を確認した。NorthflankはRunning / 1 / 1、restart 0を確認。ログ1行は5分同期の待機中で、同期完了はこの初期観測では未確認。

build、型検査、3種類のSmoke、Clippy、BOM / LF / 資料リンク検査は通過。Codexから実OCへのスタンプ試験は発行していない。配備・受信開始・模擬SDK送信を実LINEでのスタンプ受理や表示の成功とは扱わず、利用者が!id stickerで取得した少数のIDを試す。

2026-10-05、にゃんこストア更新通知を移植。追加環境変数は不要、初期登録は空。Protocol v16のNative/Adapterを同時に更新し、対象トークで!pushsetting android,iosを登録する。解除は同引数にoff、状態は!pushsetting status。基準・設定・通知は既存Core snapshotの退避対象。[状態と未確認点](../../crates/kbc-core/src/store_update/docs/STORE_UPDATE.md)。
