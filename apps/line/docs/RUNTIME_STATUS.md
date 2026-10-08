# 稼働状態の計測と表示

2026-10-09、Protocol v22。`!bot status` / `o.bot status` の実装。実LINEへの配備・表示確認は別途行う。

## 境界と関数

| 関数 | 働きと接続先 |
| --- | --- |
| readContainerResources | 非同期でcgroup v2のCPU累計・割当・メモリ使用量/上限を読み、未提供ならv1へ補完する。読込失敗・無制限はnull |
| RuntimeMonitor.initialize / refresh | ビルド情報を起動時に固定し、資源計測を起動時と既存60秒metrics Timerで更新。同時refreshは一つ。CPUは単調時計で測った前回との差分平均 |
| RuntimeMonitor.snapshot | キャッシュ済み資源値と、その時点のApiScheduler・Receiver・実際のWorker数をDTOへまとめる。ファイル読込・LINE APIなし |
| ApiScheduler.snapshot | 待機数/32、処理枠使用数/設定並列数、cooldown残り、起動後の制限検知回数。枠は間隔待ち・cooldown待ちも含む |
| createCoreの受付前update | submitBatch / submitBatchAsync前にsnapshotをNativeへJSON文字列で渡す。SDKオブジェクトを渡さない |
| NativeCore.update_runtime_status / Runtime.update_runtime_status | 入力4KiB以内、Protocolの型で読み、最新一件をMutexへ保存。DB・復旧snapshotへ保存しない |
| bot_management::quick_status / status / runtime_status | LINE照会なしでCore統計・停止設定とDTOを読み、利用者向けキーで表示。通常配送へ登録 |

既存healthと毎分metricsも同じRuntimeMonitorを使用する。CPU・メモリをstatusのたびに測定するTimer・API・Queueは作らない。状態値は受付時点、資源値は最終計測時点で、計測の経過秒を表示する。status自身の返信は表示された未完了数には含まれない。配送APIが詰まっている場合に即時送信を保証する仕組みではない。

## 資源の意味

CPUの1コア換算は累計CPU時間差分 / 経過時間 × 100。割当比はこの値をcgroupのquota / periodで割る。例えば0.2コア割当で1コア換算10%なら、割当比50%。サンプリング初回や計測元が切り替わった回は使用率未取得。quotaが無制限または読めなければ割当比は未取得とし、ホストCPU数や0.2を決め打ちしない。

cgroupが読めればCPU・メモリはコンテナを対象にし、ffmpeg等の子プロセスも含む。メモリにはcgroupが課金するファイルキャッシュ等も含まれ、プロセスRSSとは一致しない。読めなければCPUはprocess.cpuUsage、メモリはprocess RSSへ切り替え、プロセスと明記する。メモリのcontainer使用量が未取得なら、取得できた上限とプロセスRSSを組み合わせた使用率は表示しない。v2のmax、v1の負値・巨大な無制限値は未取得。メモリはMiB、使用率は小数1桁。

## 待機列の意味

Coreの未完了Actionはqueued / claimed / querying / sending / preparing / unknownの合計。共有上限は実際の受付制限MAX_ACTIONS=2,048。各待機列の分母2,048は別枠の確保数ではなく、この共有枠を使う。

queuedは期限到来分を照会（OcRequest.is_readの種別）、配送（更新APIを含む）、素材準備に分け、未来dueを予定時刻待ちへ分ける。照会中querying、配送中claimed+sendingの分母はmainの実際のloop数で、現在各2。素材保持枠はprepareMediaとattachment付きActionの未完了数 / 8で、実行数ではない。unknownも保存枠を占有する。API待機32・API処理枠はCoreと別の共通通信制御で、PUSH接続そのものはRPC枠に数えない。

参加トーク一覧件数はReceiver.listedChatsの直近成功値で、OC数ではない。接続中という状態・最終イベント受付時間だけで取りこぼしなしと判定しない。最終イベントはCore受付の最終時刻でheartbeatではない。

## 稼働ビルド

ローカルのscripts/native.cjs buildはnative/build-info.jsonへビルド時点のbranch・commit・tracked変更有無を記録する。実行中にgit HEADを読み直さず、未コミットの変更を含むビルドには表示を付ける。nativeはGit管理対象外。

DockerはNorthflankのビルド引数NF_GIT_BRANCH / NF_GIT_SHAをBOT_BUILD_BRANCH / BOT_BUILD_COMMIT環境変数へ固定する。commitの補完だけruntimeのNF_DEPLOYMENT_SHAも使用する。[公式の注入値とARGからENVへの引継ぎ](https://northflank.com/docs/v1/application/secure/inject-secrets)。他のビルド環境では同じbuild-argを渡す。branch/commitがなければ未取得で、mainや現在の作業コピーを推測しない。commitは先頭7桁、branch/commitのみ取得し、環境変数一覧や認証情報は表示しない。

Native/AdapterはProtocol v22へ同時更新する。新しい認証・secret・有料ディスクは不要。文面は[bot.txt](../../../content/messages/bot.txt)、差し込み契約は[Core schema](../../../crates/kbc-core/src/messages.schema.json)。

## 最小検証

既存Smokeへcgroup v2の0.2コア/512MiB、上限なし・ファイル欠落・v1補完、API枠2と待機6/32・cooldownの検証を追加。OC Smokeでは未完了の照会1件と待機1件を残し、statusが追加照会なしで通常配送できること、CPU割当比50%、メモリ200/512MiB、branch/短縮commit・時間分秒・共有枠表示を確認する。実コンテナの計測値・Northflank注入値は本番で確認が必要。

オフライン検証: Protocol生成、Native release build、TypeScript check/build、Core/ProtocolのClippy（all-targets・警告拒否）、Rust format、受信配送/OC/文面のSmoke、文面キー・資料リンク・BOM検査を通過。実LINEへの送信・本番配備は行っていない。
