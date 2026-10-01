# 旧Northflankサービスでの受信実験

作成日: 2026-10-01（JST）
状態: 直列取得の短時間実測・連続入力・返信有効起動を終了し、Serviceを元の停止0 / 0・Default configurationへ切戻し。全件照合・実返信は未評価。同時入力の手動試験は利用者指定で運用観測へ回す。本運用の受信方式は未確定。

## 1. 対象と確認範囲

利用者が指定した[既存サービス](https://app.northflank.com/t/kenkou-rakv0s-team/project/kbc-discord-bot/services/kbc-rakv0-line-bot/)と旧Botのアカウントを使う。新しい有料サービス・アカウントは作らない。設定を調べ、旧受信器を停止してから単一の実験受信器へ切り替える。

最新LINEJS 3.4.2の認証状態再利用、OCイベントの取得、取得ページ、同時メッセージ、CPU・Memory・API回数を先に実測する。第一回は送信なし。`o.ping` は記録上の目印で、このProbeは返信しない。Rust Core、永続Inbox、通知配送、PUSH採用の検証は別の段階として残す。

比較基準は公開 `square.fetchMyEvents` の直列取得。利用者の追加指定により既定1秒間隔、続きのページも1秒間隔。全OCの個別巡回・名前照会・既読送信・Talk受信・Command実行を行わない。認証確認にはSDKの `getProfile` を1回使い、期限切れの既存tokenはSDKの通常refresh経路が1回扱う。パスワード・QRの新規ログインへfallbackしない。

この条件でのCPU値は「受信と記録のみ」の値。旧Bot全体と同機能の速度比較ではない。1秒間隔の遅延とAPI回数の関係も記録し、本運用の間隔は結果から決める。

## 2. 配備前に管理画面で確認すること

- Service、現在のimage / deployment ID / commit、配備元と自動更新設定、entrypoint / CMDとoverride。
- 稼働replica数、CPU・Memory割当、port / health check、現在の稼働・APIエラー状況。
- 認証Storageの保存先、Volumeの有無、GitHub暗号化バックアップの設定有無。値そのものは資料やログへ転記しない。
- 旧Bot停止、切替後の実験終了待機、認証更新時の保存、元のimage・commandへ戻す手順。

管理画面へアクセスできるまで、image更新・停止・LINE接続は行わない。既存のlocal `.env` が同じアカウントとは仮定しない。

Northflankでは[command override](https://northflank.com/docs/v1/application/run/override-command-entrypoint)で起動commandを切り替えられ、[container shell](https://northflank.com/docs/v1/api/execute-command)も利用できる。実際のService設定を確認して最小の方法を選ぶ。稼働中Botへ並行してProbeをexecしない。旧imageのコードが必要な `legacy-storage.mjs` は、旧imageを保持する方法で使う。

## 3. スクリプトと上限

| ファイル / 関数 | 役割・呼出関係 |
| --- | --- |
| [live-probe.mjs](../live-probe.mjs) / `runLiveProbe` | 認証Storageコピー → API観測設定 → 既存token確認 → owner / checkpoint確認 → ページ取得 → メタデータ記録 → 通信停止・待機 |
| `numberSetting / cgroupResources / errorCode / failure` | 設定範囲、Linux cgroup v2情報、secretを含まない失敗code。`--check` はネットワークなしでSDK / Runtime / cgroupを確認 |
| `record / stop` | 件数・容量が有限のNDJSON記録、期限・signalで全HTTP通信を取消 |
| `BaseClient.fetch` wrapper | SDKの標準Node transportを保持し、全HTTP通信へ回数上限・取消を追加。HTTPエラーのstatusを記録し停止 |
| `requestCore` wrapper | 許可したRPCの回数・所要時間・失敗code。SDK内のrefresh / 再実行も数える |
| [legacy-storage.mjs](../legacy-storage.mjs) / `run` | 旧imageのStorage初期化による既存バックアップ復元。実験でtokenが変わった場合だけ認証3項目を旧Storageへ保存し、既存backupをflush |
| [container-bootstrap.template.cjs](../container-bootstrap.template.cjs) / `runNode` と起動処理 | source / lockを一時フォルダへ展開し、旧Storage復元 → latest SDK導入 → Live Probe → 認証保存を接続。失敗時はLINEへ再試行せず待機 |

- 受信時間: 既定300秒、設定可能30〜900秒。RPCとHTTPは各400回を既定上限とする。SDKの呼出とtransportの試行を別に数える。
- 各page: 100件まで。記録・重複識別用Setは最大10,000イベントに制限。ログ5MiBまで、終了記録1件を追加。
- API失敗・HTTP失敗・cursorの無進行・保存失敗で停止。制限値の探索、連続再ログイン、失敗の自動再試行はしない。
- メッセージ本文・投稿者・トーク名・token・cursorは観測ログへ出さない。トーク・message IDは実験ごとのHMACで置換。実験内での相関にだけ使う。
- `auth.json` と `checkpoint.json` は秘密情報。コンテナ内の制限した出力フォルダだけに置き、共有資料・Git・ログへ取り出さない。
- 正常終了後は `/health` の状態を `finished` にして待機し、LINE通信しない。Botのreadyとは区別する。同じ出力フォルダにある `started` により同Run IDを再実行しない。
- 再実験時はloader側から `PROBE_RUN_ID` を指定する。bootstrapはその値を検証して出力先へ使い、各観測行にもRun IDを記録する。開始前の履歴に含まれる `o.ping` と試験中の新着は時刻で分けて照合する。
- 追加実験の `PROBE_REPLY_ENABLED=1 / PROBE_COMPARE_CHAT=1` は既定無効。新着 `o.ping A01` または `o.ping B01` の最初のトークだけを試験OCにする。`o.ping` は空白を除去し、任意の `A00`〜`B99` 形式の試験番号だけをメタデータへ残す。
- `recordEvents` は通知とトーク取得を同じIDで照合し、source / pageを記録する。`sendReplies` は受信から独立した単一Workerで、有限Queue32件・累計50件まで。入力・停止で待機を起こし、各送信後1秒待つ。過去入力・別OC・重複へ返信しない。送信失敗は再投稿せず実験を停止する。
- 試験OCの通知が届いた場合だけ `fetchSquareChatEvents` を直列で追加し、イベントが残れば次の取得区間で継続する。100件/page・40回まで。全OCを常時巡回しない。RPC・HTTP各400回の共通上限に比較と送信も含む。これは本運用の完全なAPI Schedulerではない。
- 送信時にLINEJSが保存する `reqseq` の `sq` も、終了後に旧Storageへ単調増加で引き継ぐ。認証が変わらなくても保存・既存backup flushを行い、他のsequence・旧checkpoint・機能データは維持する。
- Volumeがなければコンテナ交換でmarker・認証コピー・結果は失われる。配備前に確認し、実験終了前の再配備や自動更新を避ける。既存Serviceのrestart方針を未確認のまま「再接続しない」とは保証しない。

## 4. 実行・観測・切戻し

実行場所は、旧Botの本体を起動しない状態の既存image内。実験packageは別フォルダへ置き、旧SDKのnode_modulesを上書きしない。`npm ci --ignore-scripts --no-audit --no-fund` でlock通りに導入する。

以下はコンテナ内の実験フォルダに置いたscriptを実行する形。今回の配備方法と保存場所は5節に記録する。

```sh
node legacy-storage.mjs restore --old-receiver-stopped
node live-probe.mjs --check
PROBE_RUN_ID=receiver-20261001-a node live-probe.mjs --live --old-receiver-stopped
```

環境変数 `LINE_DEVICE / LINE_STORAGE_FILE / LINE_AUTH_TOKEN` は元の設定を継承。`LEGACY_APP_DIR` の既定は `/app`。必要に応じて `PROBE_DURATION_SECONDS / PROBE_INTERVAL_MS / PROBE_MAX_REQUESTS / PROBE_OUTPUT_DIR` を設定する。旧アカウントのcheckpointとownerが一致しなければ停止し、cursorを黙って初期化しない。

1. 旧BotのCPU・Memoryとエラー状況を記録し、旧受信器を止める。replicaが複数なら全て止まったことを確認する。
2. 既存認証を復元してコピーし、latest SDKのprobeを1接続で起動する。認証失敗は原因codeを記録して終了する。
3. 無通信時の計測後、自然に受信したイベントとAPI回数を記録する。当初の同時 `o.ping` による手動照合は、利用者の最新指定で今後の運用観測へ回す。必要な試験入力を使う場合は条件と件数を記録し、既存の概数入力を全件照合済みとは扱わない。
4. `observations.ndjson / summary.json` のメタデータだけを保存する。古いcheckpointから返った過去イベントと試験中の入力を時刻で分ける。自然なバックログでページ継続を観測し、大量投稿で強制しない。
5. `authUpdated=true` なら、Probe停止後・旧Bot再起動前に `PROBE_OUTPUT_DIR` を明示して次を実行する。

```sh
node legacy-storage.mjs save-auth --old-receiver-stopped
```

認証の `.auth / refreshToken / expire` だけを保存する。checkpointは旧Botのものを維持する。既存GitHub backupを利用する場合はflush成功を確認してからコンテナを交換する。backup未設定でVolumeもない場合は、認証コピーを失う切替方法を先に解決する。

6. 元のimage・command・稼働数へ戻す。今回は開始前が停止状態なのでreplica 0へ戻す。旧checkpointからの再取得や旧Runtimeの過去メッセージ除外を踏まえ、実験中の旧Command実行・返信を保証しない。

## 5. 実施状況

2026-10-01: latest SDK固定・構文確認・`--check` をWindows / Node.js v24.15.0で実行。`network=false`、cgroup情報は取得不可。その後、利用者がログインしたChromeで既存Serviceを確認し、以下の実LINE実験を実施した。

同日、既存SDKの `RequestClient.requestCore` だけを模擬応答へ置換する一時scriptで、Live Probeの記録・停止経路を確認。2ページ・2トーク・異なる3 IDと重複1件を区別し、次の模擬API制限codeで停止した。原本Storageを変更せず、観測ログにfixtureのtoken・本文・トークID・cursorを含めないことと、終了後 `/health` が `finished` を返すことを確認した。外部通信0回、local health確認のみ。試験用script・fixtureは回収し、恒久Test Frameworkは追加していない。これは実LINEでの制限発生や取りこぼし検証ではない。

認証切戻しhelperも模擬の旧Storageで確認。認証3項目だけを更新し、checkpoint・機能データを維持した。owner不一致では変更前に停止し、backup未設定ではflush成功を報告しない。実サービスでは旧imageのStorage初期化・既存認証の再利用が成功した。実験中の認証更新はなく、認証の切戻し保存は不要だった。

同日、利用者がChromeでログイン後、指定Serviceの管理画面を確認した。

| 項目 | 開始前の状態 |
| --- | --- |
| 稼働 | Paused、0 / 0。旧受信器は起動していない |
| Compute | nf-compute-20、0.2 vCPU / 512MB Memory / 1024MB ephemeral storage |
| 配備image | `festive-twig-6592`、旧 `main` の `c6796e05ad76397acbd1e17f0635924e08f1d54e` |
| Working directory / command | `/app`、Default configuration、`docker-entrypoint.sh node dist/main.js` |
| 自動配備 | New builds will not automatically be deployed |
| Volume / Runtime files | どちらも未追加 |
| 保存設定 | GitHub repo / branch / token、暗号化Storage path / keyの設定名を確認。値は転記しない |

Chromeのファイル転送は利用できなかったため、CMD overrideのCustom commandへ、公開source取得用の小さなloaderをbase64で渡した。Runtime fileは保存していない。公開[新リポジトリ](https://github.com/sinsuirakv0/KBC-rakv0-LINE-bot-v2)の固定commit `0d3327bb9599ce9c502b8f2be3848ed525aead61` から5 assetとbootstrap templateを取得・再構成し、SHA-256 `c963ad18347af937d11f897aa87568e993aaecbabb503892e17c5cf5da7c40e7` を検証して起動した。旧image・既存環境変数を継承し、実験依存は `/tmp/kbc-linejs-receiver` へ分離した。

Run IDは `receiver-20261001-a`、コンテナ内出力は `/app/logs/receiver-probe-receiver-20261001-a`。既存Storageを復元した後、replica 1の単一受信器を起動。受信終了後は通信せず待機し、`bootstrap-finished` を確認した。最後にreplica 0へ停止し、CMD overrideをDefault configurationへ戻した。旧Bot本体の再起動は行っていない。

## 6. 実測結果と判断

公開するのは[集計JSON](../results-live-20261001.json)。Northflank画面のメタデータログから集計し、認証・checkpoint・本文・トークID・message IDは取り出していない。

| 項目 | 実測 |
| --- | --- |
| Runtime / SDK | Linux、Node.js v24.21.0、LINEJS 3.4.2 |
| 計測期間 | 2026-10-01 20:06:27.969〜20:11:27.967 JST、約300秒 |
| 取得条件 | 各取得後1秒待機、100件/page、RPC・HTTP各400回まで、送信なし |
| API | `getProfile` 1回、`fetchMyEvents` 257回。HTTP 258回。期間終了で正常停止、API制限・API失敗は観測されず |
| 受信 | 4トーク、15イベント、6メッセージ、重複0。メッセージの4件は開始前の履歴、2件は実験中の新着 |
| 新着のlag | 565ms / 773ms。外部作成時刻との差で時計ずれを含む。2件だけなので応答性能の代表値にはしない |
| 取得RPCの時間 | 後半127回の観測分: p50 156ms、p95 161ms、最大288ms。全257回の集計ではなく、返信時間でもない |
| Probe CPU | 起動直後2区間を除く17区間: 1コア比0.466〜1.013%、平均0.636%。0.2コア割当比では2.33〜5.06%、平均3.18% |
| コンテナMemory | 同17区間のcgroup current: 138,502,144〜152,576,000 bytes（132.1〜145.5MiB）。Probe RSSは98.2〜110.6MiB |
| 実際の資源上限 | cgroup `cpu.max=20000 100000`、`memory.max=512000000`（488.3MiB）。管理画面の512MBとMiBを区別 |
| 認証と復旧 | 旧認証・owner・checkpointを再利用成功。`authUpdated=false`。原本checkpointは変更せず、停止 0 / 0・Default configurationへ復帰 |

CPUはProbeプロセスの `process.cpuUsage()`、Memoryはコンテナ全体のcgroup currentとProbe RSSを分けている。開始直後のcgroup currentは約223MiBだったが、その後132〜145.5MiBとなった。短時間でRSS・currentが増えており、長時間での頭打ちは未確認。旧サービスは開始前から停止していたため、旧BotのCPUとの差は測定していない。

各取得の完了後に1秒待つため、通信時間を含む開始間隔は約1.16秒。OC別巡回なしで4トークのイベントを受けたことは確認できたが、参加OC総数・多OCでの保証・サーバーのAPI上限は判定できない。短時間で制限がなかったことを、送信・照会・背景処理を併用する本運用へ一般化しない。

試験入力 `o.ping` は0件で、利用者からの送信件数との照合は実施できていない。自然な新着2件を、ほぼ同時入力の全件受信の証明にはしない。100件超のページ、長時間・切断・再開・実制限後の復帰、HTTP/2 PUSH、永続Inbox、通知配送も未確認。次は受付・checkpoint・復旧契約を具体化する。同時入力の手動照合は後続の運用観測へ回す。

## 7. 2人による連続入力の実測と次の比較

Run `receiver-20261001-b`、source `c118725745d002470cf4a4c7cb0c70241e9cbc4e`、bundle SHA-256 `7e81f17ef9f587ff08e29eba384ec307e1b32bcb11f6cef62c357f84e2d7dd8c`。20:45:21.985〜20:50:21.902 JSTの約5分、取得後1秒・送信なし。258回の取得・HTTP259回で正常終了し、4トーク・75イベント・異なる63メッセージ・重複2件を記録した。新着59件のうち試験トーク45件、他トーク14件。完全一致 `o.ping` は38件で、利用者の送信概数は「50くらい、数え間違いの可能性あり」。7件の他の本文は記録していないため、空白付き入力等だったかは判断できない。全件受信・欠落件数は未判定。

`o.ping` のlagは122〜1197ms、p50 606ms、p95 1175ms。CPUは起動直後2区間を除く17区間で1コア比平均0.458%、割当比2.29%。cgroup currentは134.3〜143.8MiB。API制限は観測されなかった。全65件のメッセージイベントは `NOTIFICATION_MESSAGE` で、受信したpingの時刻差は最短約1156ms。通知から全メッセージが得られるという保証は確認できていない。

SDKは[アカウント側の通知](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/client/client.ts)と[トークのイベント取得](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/client/features/square/mod.ts)を別に扱う。この結果から、試験OCに限定した `fetchSquareChatEvents` と通知のmessage ID照合を次の比較に追加する。通知の集約・保持やProbeの完全一致判定は仮説として残し、欠落原因が確定したとは扱わない。

利用者の追加指定により `pong` 返信も準備した。番号付き20入力（A01〜A10、B01〜B10）で受信経路・番号・ID・API送信成功を照合する。API成功と利用者の画面での受信は区別する。模擬RPCで重複1件を1返信にまとめ、過去・別OCへの返信を除外し、Queueが空になった後の追加入力も配送した。3件の返信、Square sequenceの保存・引継ぎ、元の認証・checkpointの維持を確認。外部通信なし。模擬script・fixtureはファイル単位で回収し、恒久Test Frameworkは追加していない。

続いてRun `receiver-20261001-c`、source `dc6773755c814734110ae0d3bfe4f552ab0ccf25`、bundle SHA-256 `fba612a61b23b231b760d40930d9e5ed254fefe17ee80d0d8183c57ea324f8fb` で、返信・トーク比較を有効にして起動。21:11:25.765〜21:16:25.766 JST、255 RPC / HTTP、254取得page、22イベント・12メッセージ、4トーク、API失敗の観測なしで期間終了した。認証とcheckpointの再利用、返信有効設定を実コンテナで確認した。

この期間は新着 `o.ping` が0件で試験OCを確定できず、`fetchSquareChatEvents / sendMessage` は各0回。実LINEでの返信・通知とトークの比較・20番号の照合は未評価。認証更新・送信sequence変更はなく、終了後は元の停止0 / 0・Default configurationへ戻した。

利用者の最新指定により、重なった入力への返信抜けを再現する手動試験・番号照合は今後の運用観測へ回す。試験完了を待って設計を止めず、[PUSH受信の範囲](../../../docs/research/PUSH_RECEPTION.md)と受付・復旧契約の設計へ進む。未評価を成功へ変更せず、受信・処理・送信のID追跡を後続へ引き継ぐ。
