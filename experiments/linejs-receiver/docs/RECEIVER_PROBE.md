# LINEJS 3.4.2 受信Probe

実施日: 2026-10-01（JST）
状態: 最新配布物での認証なし検証を完了。SDKの観測結果であり、新Botの受入試験・実LINE確認ではない。

## 1. 対象と条件

- SDK: [最新公開版3.4.2](https://jsr.io/@evex/linejs)。`package.json` はexact指定し、`package-lock.json` に依存全体を固定。
- 環境: Windows、Node.js `v24.15.0`、Undici `7.30.0`。0.2コア・512MBに制限した環境ではない。
- 配布物: `https://npm.jsr.io/~/11/@jsr/evex__linejs/3.4.2.tgz`。取得integrityは[結果JSON](../results.json)に記録。
- [実行コード](../probe.mjs)は、配布物の実際のStream、Polling、Client、PUSH callback、LEGY、Node transportを呼ぶ。SDKを改変していない。
- イベントとAPI応答は合成。認証情報・旧Storage・実メッセージを使わず、LINEへ接続していない。
- 実通信は `127.0.0.1` のHTTPとTLS応答を返さないTCP serverだけ。SDK fetchに外部URLが渡ったら検証を失敗させる。
- 新prefix `o.` を検証入力 `o.ping` に使用。Command parserはまだ実装していない。

## 2. 観測結果

| 条件 | 結果 | 判断 |
| --- | --- | --- |
| 同じ本文・時刻、異なるID、3トークの500件を読取前に投入 | 500件すべてを投入順に読取 | Stream単体では2件以上の同時相当入力を潰さない。全経路の無反応の原因確定ではない |
| 500件投入後のrenew | 新Streamに300件（ID 200〜499）。旧Streamには読める200件が残る | 旧内部Queueは引き継がれない。別配列のため200は総Buffer上限ではない |
| 500件投入後のcancel / error | 新Streamは同じ300件。旧内部Queueの200件は読取不可 | SDK内部Queueだけを未処理の保存場所にしない |
| PUSH通知1回、250件分の模擬バックログ | 100件を1回取得し、応答にあるcontinuationを追加取得しない | この関数単体で全ページ排出を保証しない。実LINEが後続PUSHを送る条件は未確認 |
| 同じ250件を旧方式のSDK generatorへ模擬ページとして返す | 100・100・50件の3ページ、合計250件を取得 | ページ継続の比較基準にできる。ただしdeprecated generatorを製品へ直接採用する判断ではない |
| PUSH応答のcursorと読取の順序 | consumerの読取前にcursorがcheckpoint-1へ更新され、update:syncdataを発行 | 永続受付済みのcheckpointとしてそのまま保存しない |
| PUSH取得を2回同時開始し、2回目→1回目の順に模擬応答 | 両方とも同じ開始cursor。後から完了した1回目の値が最終cursorを上書き | SDKに取得の直列化・古い完了結果の抑止がない。実サーバーでこの順序やcursorの差が発生した証拠ではない |
| 高水準Clientで2件を配送 | raw・square:messageとも2件を配送 | 基本経路は成立 |
| 1件目のraw listenerが同期例外 | 2件目は配送。1件目のsquare:messageは発行されない | 3.4.1の継続修正を確認。受信listenerを軽量・非throwにし、業務失敗を別経路で扱う |
| 非同期raw handlerの1件目を保留 | 2件目は先に開始・完了。完了順は2→1 | SDKはasync handler完了を待たない。OCの状態・対話の順序はCoreが管理する |
| PUSH初期化失敗後、新しいlistenを開始 | 両共有readerへ元のエラーが伝わり、islistenが戻る。2回目でイベント取得 | 初期化失敗の改善を確認。単一のSupervisorが再開を所有する |
| PUSH callback中の取得が模擬APIエラーでreject | unhandledRejectionを観測 | 最新版でもPUSH callbackの失敗を捕捉する対策が必要 |
| PUSHのresponse streamを1,200ms後に返す模擬通信 | 約813msでno resStream。closeの取消がmockへ到達 | 固定の300ms開始待ち＋500ms再確認では遅い準備を待てない。旧15秒パッチの目的は残るが、同じ置換を採用した状態ではない |
| 暗号化LEGYのsignal取消 | 外側Requestへ伝わり、mockはAbortErrorを返す | 旧signal追加パッチの目的は最新版に含まれる |
| custom fetchで通常RPCとPUSHを呼ぶ | 両方が同じcustom transportを通る | 標準HTTP/2 PUSH transportを置き換える点を実行確認 |
| Node RPCをloopbackで開始して取消 | AbortError。server側でもrequest abortを観測 | この環境では取消が実接続へ届く |
| Nodeの通常 / PUSH transportでTLS準備を停止 | 設定100msに対し実測950 / 1,026msでTypeError、causeはUND_ERR_CONNECT_TIMEOUT。cleanup前のsocket残数は0 | 接続timeoutと切断を確認。ただし100msを厳密な終了期限と解釈しない |

最後のTLS検証は接続準備の確認であり、TLS確立後のHTTP/2 PUSH配送・ALPN・実LINE切断を検証したものではない。Undiciの低精度Timerの実装も確認した。時間値は手元の一回分で、Northflankの性能基準ではない。

PUSH callbackの観測では、`Conn.onPacketReceived` が `manager.onPushResponse(packet)` のPromiseをawait / catchせず、`_OnPushResponse` 内の `fetchMyEvents` のrejectが外へ出ることを、配布物の実関数で再現した。検証プロセスだけにunhandledRejection observerを置いて結果を記録した。このobserverをBotの対策として使う提案ではない。

## 3. 次の設計へ引き継ぐ条件

最新3.4.2のSDK既定PUSHを、そのまま第一段階の確実な受付器として採用する条件は満たしていない。PUSHの本採用は保留し、受信方式の比較候補として残す。

次の受付・復旧設計では、取得をアカウント単位で直列化し、全ページを扱い、Batchとcheckpointを永続受付してから確定する経路を先に定義する。比較基準は最新SDKの公開 `square.fetchMyEvents` を利用する案とする。旧版の固定1秒・OC別2秒巡回をそのまま再採用する決定ではない。

PUSH案は、Buffer有限性、待機中Eventの再取得、ページ継続、callbackのreject捕捉、遅い接続準備、実通信の取消を補える場合に比較する。SDK補完や最小修正の維持負担と、実LINEでのAPI回数・CPU・受信lagの改善を記録して採否を決める。今回の検証ではSDK・製品の受信層を修正していない。

LINE側の保持・subscription・再通知・cursorの意味、API制限のcode / scope / 上限、0.2コアでのCPU、通知停滞の旧Runtime原因は未確認。実LINEで確認すべき項目と、次のCore / ApiScheduler検証を分けて残す。旧Botの障害原因を今回のSDK観測だけで確定しない。

## 4. 関数と再実行

| 関数 | 役割・呼出関係 |
| --- | --- |
| `streamBoundaries` | 実SDKのStreamへ合成Eventを入れ、`readEvents` でburstとrenew / cancel / errorを確認 |
| `pagesAndCursor` | `_OnPushResponse` と `Polling._listenSquareEvents` へmockページを返し、取得回数・cursor・同時完了を記録 |
| `clientDelivery` | 実Client.listenへmock Streamを渡し、rawと高水準配送、同期例外、非同期完了順を観測 |
| `startupAndPushError` | 初期化だけをmock化し、実Pollingの再開と実Conn callbackのrejectを確認 |
| `pushReadiness` | 実Conn.new / read / closeへ遅いmock responseを返し、準備待ちと取消を確認 |
| `cancellationAndTransport` | LEGY・custom fetch・SDK Node transportをmock / loopbackで確認 |
| `eventAt / messageId / makeWriter / readEvents / record` | 合成入力、ID抽出、実SDK Stream、少数の共通読取と結果集約 |

再実行はこのフォルダの親 `experiments/linejs-receiver` で行う。

```powershell
npm.cmd ci --ignore-scripts --no-audit --no-fund
npm.cmd run probe
```

`npm ci` は公開レジストリから依存を取得する。Probe実行はLINEへの外部通信を禁止し、25秒の上限を置く。終了時はloopback server・socket・SDK dispatcherを回収する。結果JSONは再実行で更新される。

これはPhase 0用の一つの調査scriptで、恒久Test Frameworkは追加していない。assertionは既知の危険な挙動も再現できたことを確認するためのもので、すべての受入条件に合格したという意味ではない。
