# LINEJS受信・常時処理の調査と比較実験

調査日: 2026-10-01〜2026-10-02（JST）
状態: 実装調査、最新3.4.2配布物の認証なしProbe、既存コンテナの直列取得実測を実施。実LINEのPUSH比較は未実施。同時入力の手動試験は利用者指定で運用観測へ回す。

## 1. 判断

比較実験は必要。旧版の固定周期による取得に対して、LINEJSにはOpenChatのPUSH通知を起点とする取得経路があり、無通信時のAPI呼出・CPUや受信遅延を改善できる可能性がある。

SDK内部のBuffer、cursor更新、再接続、ページ継続、イベント抽出を確認する必要がある。初回調査ではPUSH採用を保留したが、2026-10-02に利用者指定で[PUSHを基本に有限並列を使う設計方針](../decisions/PUSH_AND_BOUNDED_CONCURRENCY_V1.md)を採用した。SDK既定ループへの対策と実LINEの検証は残る。

## 2. 対象と一次資料

- 旧Bot: `D:/KBC/KBC-rakv0-line-bot` 、HEAD `c6796e0` 。
- 手元SDK: `node_modules/@evex/linejs` 、 `3.1.4` 。既存のLEGY / PUSHパッチ適用状態を含む。
- 最新公開版: [JSRの `3.4.2`](https://jsr.io/@evex/linejs) 。[Release](https://github.com/evex-dev/linejs/releases/tag/v3.4.2)は2026-09-07 08:56:38 UTC公開。
- 開発元の調査commit: `ef6c3d9f70dd41fa51053615d47f071f58cf8db3` 。最新 `v3.4.2` のReleaseが指す `ef6c3d9` と一致。後述の10節で配布物そのものの実行検証を追加した。
- [Polling実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/polling/mod.ts)
- [PUSH・Stream実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/push/connManager.ts)
- [Client.listen実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/client/client.ts)

手元 `3.1.4` の実行ファイルと、最新 `v3.4.2` に対応する開発元の固定commitを別々に確認した。比較時は採用versionとlockを記録する。以下のE1 / E2は手元 `3.1.4` での実行結果であり、最新版の配布物による実行結果とは区別する。

## 3. SDKから分かったこと

| 確認内容 | 比較実験への影響 |
| --- | --- |
| `listenSquareEvents()` はLEGY PUSHとSquare Streamを使う | 関数名だけで定周期pollingと判断しない |
| Square PUSHの通知後にも `fetchMyEvents(limit: 100)` を呼ぶ | PUSHはAPI取得が不要になる方式ではない |
| その取得で `subscriptionId` 、 `syncToken` を使う | 新規購読・再接続・再取得の契約を確認する |
| SDK側でcursorを更新し、 `update:syncdata` を発行する | Rustの永続受付前に確定checkpointとして保存しない |
| Streamの `highWaterMark: 200` を超える分を別配列へ保持する | CoreのQueueだけを有限にしてもSDK全体の上限にはならない |
| `Polling.listenTarget` の既定は `[3, 8]` | OC専用の比較で不要なTalk購読・内部通信を始めないか確認する |
| 手元3.1.4の高水準 `square:message` は `NOTIFICATION_MESSAGE` を抽出する経路 | rawの `square:event` とメッセージ・thread・参加退出の網羅性を比較する |

調査したPUSH取得箇所には、 `continuationToken` を使って追加ページを排出する処理が見当たらない。これだけで欠落と断定せず、100件を超えるバックログの継続処理を比較条件へ入れる。

## 4. 実施したオフライン実験

環境: Windows、Node.js `v24.15.0` 、手元LINEJS `3.1.4` 。
方法: 実際の `ConnManager.prototype.createAsyncReadableStream` を直接呼び出す。認証・Client接続・LINE API呼出・実メッセージは使用していない。

### E1: consumerを遅らせて500件を投入

異なるID、同じ本文・時刻を持つ500件を、読取開始前にenqueueした。

- 新しい読取処理は500件すべてをID順に取得した。
- `highWaterMark: 200` より多く投入しても受付は止まらなかった。
- ソースの別配列には件数上限の検査がない。200はBuffer全体の最大容量ではない。

これはSDK Stream単体の結果であり、LINEからの同時メッセージやCommand処理の成功を証明しない。

### E2: 500件を投入してからStreamをrenew

読取開始前に500件をenqueueし、 `renew()` 後のStreamから読む。

- 新Streamから得られたのはID 200〜499の300件。
- 旧Streamの内部Queueに入っていたID 0〜199は、新Streamへ引き継がれなかった。
- 旧Streamも別途読めばよい場合と、接続取消で破棄する場合を区別する必要がある。

この結果は「新Streamへの引継ぎ」の確認であり、旧Botで報告された無反応の原因確定ではない。PUSHを候補にする場合、renew / cancelと永続checkpointからの再取得を組み合わせて検証する。

再実行の要点は、SDKの上記関数で作ったWriterへ連番500件を入れ、Aでは同じStreamを500件読む、Bでは `renew()` 後のStreamを300件読むこと。APIを呼ぶ `initializeConn()` やClientログインは不要。小さな一時検証で済むため、専用Frameworkや恒久テストは追加しない。

## 5. 次に必要な比較

| 比較 | 調べること | 実施条件・採用判断 |
| --- | --- | --- |
| E3: 旧raw取得とOC PUSH | 無通信API数、CPU、受信lag、同時入力、切断復帰、Buffer・checkpoint | 先にSDKの有限性・再開をオフライン確認。採用版と少数の実験OCを固定して順番に比較 |
| E4: 全体イベントとOC別補助取得 | 参加退出・メンバー更新・thread等の網羅性、重複 | 同じ事象のIDと種別を突合。全体イベントで足りる種類だけ補助取得を減らす |
| E5: 定期名前巡回と受信・期限による補完 | 名前の変更履歴の要件、照会回数、stale期間 | 未取得・期限切れ・必要な表示を起点にする案と比較。履歴要件を勝手に落とさない |
| E6: 定期通知確認と最短期限Timer | 無入力でも通知が届くか、再起床、時計変更、再起動 | 仮想時刻・API mockで先に確認。通知遅延が要件内でAPI・CPUを減らせるか判断 |
| E7: 背景処理の固定周期とdirty / 状態変更 / 期限 | 保存・GitHub同期・監視の呼出数と負荷 | 変更時の処理集約と最大未同期時間を比較。必要なhealth監視は維持 |

SDKの100件取得境界と200件Stream境界は、オフライン再生で先に確認する。本番へ大量のメッセージやAPI要求を送って制限を探る方式にしない。同じアカウントの二重ログインも避ける。

## 6. 常時処理の現状と扱い

| 旧実装 | 現在分かったこと | 判断 |
| --- | --- | --- |
| `main.ts: listenRawSquareEvents` | 通常1秒、ページ継続25msの待機。実通信待ちは別 | PUSHとの比較対象 |
| `ocJoinMessagePolling.ts` | OCごとに `fetchSquareChatEvents` 、次回期限は2秒、設定対象は60秒で再解決 | raw全体受信で補える種類と不足する種類を先に分類 |
| `nameHistory/store.ts: scanKnownSquareNames` | 保存済みSquareを巡回し、最大20ページのメンバー検索 | 受信情報・必要時照会・期限付き補完との比較対象 |
| `ocProfileStatus.ts` | bind / 状態変更で更新。表示名が同じならAPIを呼ばない | 現時点で定期更新へ作り替えない。起動時のOC列挙・変更数だけ観測 |
| 通知・リマインダー | Timerとrunning等の状態を持つ | 期限起床と配送枠の進行を別々に検証 |
| 保存・同期・受信health | 周期処理でも目的が異なる | dirty集約・期限監視・復旧判断を分け、単にTimerをすべて消さない |

## 7. 結論の記録

各比較はAPI回数、受信・通知の遅延、受付件数と未処理数、CPU、Memoryを同じ条件で記録する。実験対象・未確認点・採否とトレードオフは[文書運用](../engineering/DOCUMENTATION.md)に従う。

初回調査の結論は「PUSHと補助巡回削減を比較すべき」「SDKのStream上限・renewとcheckpointは先に検証すべき」。その後PUSHを基本の受信方式として設計採用したが、実LINEで安定性・省CPUを比較確認した状態ではない。

## 8. 最新公開版の改善と採用方針

2026-10-01時点の最新公開版は `3.4.2` 。利用者の指定により、新Botは最新公開版を基準にする。実装開始・依存更新時には公開版を再確認し、具体的なversionをlockして比較条件と運用版を記録する。

### 旧3.1.4からの主な改善

| 版 | 確認した改善 | 今回の扱い |
| --- | --- | --- |
| [3.3.3](https://github.com/evex-dev/linejs/releases/tag/v3.3.3) | `BaseClient.config.timeout` をNode RPCとHTTP/2 PUSHのTCP / TLS接続へ適用。暗号化LEGY通信へ取消signalを伝搬し、PUSH接続失敗の未処理rejectを対処 | 無期限のAPI待ち・通知枠の停滞を調べる基準にする。取消後に実通信・実行枠が解放されるか確認 |
| [3.4.1](https://github.com/evex-dev/linejs/releases/tag/v3.4.1) | listen / pusherの分離Promiseの失敗を捕捉。単一イベントの同期listener・復号失敗後もTalk / Square配送を継続。初期化失敗時にpolling状態を戻し、共有Streamへ元のエラーを伝える | エラー後の受信停止を改善できる修正として活用。終端エラーの復帰とasync handlerのrejectはAdapterでも扱う |
| [3.4.2](https://github.com/evex-dev/linejs/releases/tag/v3.4.2) | E2EE動画の送信metadataへ呼出側が指定するdurationを反映 | 最新版に含まれる修正として採用。第一段階のOC受信・負荷制御の解決とは分ける |

3.4.1にはreactionの `reqSeq` 初期化・保存を直列化する修正もあるが、通常のOC Commandの同時受信が直った証拠にはならない。初期接続の終端失敗は自動再試行されず、呼出側が新しいlistenを開始する必要がある。async event / log listenerのrejectも呼出側の責務として残る。

### 旧パッチの再評価

旧 `scripts/patch-linejs-legy-signal.mjs` は、LEGY外側Requestへのsignal追加、PUSH接続Promiseのcatch、response stream待機の500msから最大15秒への延長を行う。

- [最新版のLEGY実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/request/legy.ts)には `signal: request.signal` があり、signal追加パッチの目的は取り込まれている。
- [最新版のPUSH接続実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/push/conn.ts)には接続失敗のcatchとloggerの同期例外保護がある。
- 同じPUSH接続実装の `read()` は、response streamが未準備なら500ms待って再確認する構造。旧版の15秒待機への変更は同じ形では取り込まれていない。0.2コア環境の遅い接続で必要か、取消・timeout・再接続と合わせて検証する。

最新版をそのまま使う構成を基準にする。旧パッチの文字列置換を新版へそのまま移さず、再現した問題に必要な対策だけを検討する。

### 最新版にも残る確認

調査commitの `createAsyncReadableStream` は、200件のStream内部Queueに加え、件数上限のない別配列へ保持する構造である。`renew()` は別配列を再利用するが、旧Stream内部のQueueを新Streamへ移していない。旧3.1.4のE1 / E2で確認した性質に対応する構造が、最新3.4.2のソースにも残る。これは初回のソース調査結果であり、後の10節で配布物によるrenew / cancel / errorの確認を追加した。

初回の最新ソース取得はGitHubへの接続timeoutで失敗し、Stream検証に到達しなかった。その後、公開レジストリから3.4.2配布物を取得してlockし、10節のProbeで同じStreamの性質を確認した。初回の失敗と、後の配布物による実行結果を区別する。

PUSHでも `fetchMyEvents(limit: 100)` を使い、SDKが永続受付より先にcursorを更新する。100件超の継続取得、有限Buffer、永続checkpoint、同時入力、再接続後の再取得は引き続きE3で確認する。今回確認したReleaseからは、OCのサーバー側API制限値や、全APIの制限回避を保証する変更は確認できない。共通のAPI予算・cooldown・不要巡回の削減を設計する方針を維持する。

開発元の3.4.1 / 3.4.2は実LINEアカウントでの検証を行っていないと記録している。また3.4.2には、Windows / Node 24のRPC取消テストがローカルで `TypeError` になった未解決記録がある。採用Runtimeで遅い通信・取消・終了時の挙動を確認し、エラー名だけで取消判定を固定しない。

## 9. API観測と制御を入れる接点

2026-10-01に最新3.4.2に対応する[BaseClient実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/core/mod.ts)と[RequestClient実装](https://github.com/evex-dev/linejs/blob/ef6c3d9f70dd41fa51053615d47f071f58cf8db3/packages/linejs/base/request/mod.ts)を確認した。以下は初回のソース確認。custom fetchでRPC / PUSHが同じ経路を通ることは、後述の10節で配布物でも確認した。実LINEでの確認は行っていない。

- `BaseClient.fetch` はcustom fetchを指定するとそれを優先し、未指定ならSDKのNode transportを選ぶ。
- `BaseClient.fetchPush` はcustom fetchがあると `this.fetch` を使う。未指定ならHTTP/2用のNode PUSH transportを選ぶ。
- したがって、API観測・制御のためにcustom fetchへ単純な `globalThis.fetch` wrapperを渡すと、SDKが選ぶPUSH transportも置き換わる。PUSH側のHTTP/2・streaming・取消・接続timeoutの契約を保持できるか先に検証する。
- RequestClientはmethod名・pathを把握するが、fetch境界では暗号化されたRPCも通る。すべてをfetchのURLだけでCommand・API種別へ正確に分類できるとは扱わない。

最初の検証では通常RPCと持続するPUSH接続を分け、観測・制御を差し込む場所を決める。受信接続が返信の通信枠を占有せず、SDK内部要求・再試行も観測できることを確認してから、ApiSchedulerの接続方法を確定する。SDKのtransportを失う設計や、二重の待機列・再試行を加える設計にしない。

## 10. 最新配布物での受信Probe（着手順A）

[詳細結果・関数の関係・再実行手順](../../experiments/linejs-receiver/docs/RECEIVER_PROBE.md)と[観測JSON](../../experiments/linejs-receiver/results.json)を記録した。LINEJS 3.4.2、Node.js v24.15.0、Undici 7.30.0をlockした環境で、認証なしのmockとloopbackを使って実施。新prefixは `o.` 、検証入力は `o.ping` 。

同じ本文・時刻の500件を全件読取できた。一方、Stream更新・取消時の200件の引継ぎ不足、PUSH取得の継続ページ未排出、同時取得のcursor上書き、PUSH callbackの未処理reject、遅いresponse stream準備の失敗をSDKの実関数で観測した。模擬サーバー条件を実LINEの挙動や旧Bot障害の原因と読み替えない。

初期化失敗後の再listenとLEGY signal伝搬は確認できた。Node RPCの実接続取消とRPC / PUSHの接続timeoutはloopbackで確認した。SDK既定PUSHの本採用は保留し、次は直列なBatch取得・全ページ・永続受付・checkpointの契約を定義する。

着手順Aの認証なし確認を完了とする。実LINEの保持・再通知・制限、HTTP/2確立後のPUSH配送、0.2コアの負荷・CPUは着手順Dでの確認として残る。PUSHの設計採用後も、SDK対策・本運用の検証とPhase 0全体は未完了。

## 11. 既存コンテナでの短時間受信実測

同日、利用者の指定した旧Northflankコンテナとアカウントで、最新LINEJSの公開 `square.fetchMyEvents` を直列取得した。[条件・集計・切戻し](../../experiments/linejs-receiver/docs/LIVE_CONTAINER_PROBE.md#6-実測結果と判断)に詳細を残す。取得後1秒待機・5分・送信なしで、4トークの15イベント・6メッセージ（新着2件）を記録し、257回の取得中にAPI制限は観測されなかった。開始前が停止状態のため、旧BotのCPUとの差は測定していない。

既存認証・owner・checkpointの再利用と、0.2コアでの受信のみのbaselineを確認した。同時入力の送信件数照合は今後の運用観測へ回す。保持・再通知・制限後の復帰、長時間Memory、多OC負荷、PUSHとの比較は残る。短時間の成功を受信・返信・通知の完全性へ読み替えず、永続受付と復旧契約を次に具体化する。

## 12. PUSHの受信範囲と作業順の更新

[PUSH受信で得られる情報](PUSH_RECEPTION.md)に、持続HTTP/2接続 → 更新通知 → `fetchMyEvents` の関係、参加・退出・名前・権限・ノート・スレッド等の型、概要と詳細、高水準handlerの抽出範囲を整理した。参加者の詳細が全体取得で届かず、トーク別取得で得られた旧運用記録も引き継ぐ。型の存在を実LINEの配信保証と読み替えない。

利用者の最新指定により、重なった入力への返信抜けを再現する手動試験は運用観測へ回す。2026-10-02にPUSHを基本の取得起点として採用し、同じcursorの直列化と独立した処理の有限並列を[決定資料](../decisions/PUSH_AND_BOUNDED_CONCURRENCY_V1.md)へ記録した。イベント網羅性・受付・checkpoint・復旧を次に設計する。通知を起こさない参加イベントがある場合の補助取得は、必要な機能と対象トークへ限定する。

## 13. 本環境のトーク補完と購読情報

2026-10-03、a1730c6の本環境でPUSH受信とログ追記は成功した一方、復元した補完待ち1トークにTypeErrorを観測した。エラー名だけでは原因を確定していない。drainChatの応答subscription参照に省略時の検査がなく、LINEJS 3.4.2のSquareChat.listenは購読情報を要求せずsyncTokenとeventsでトークを取得していることを再確認した。

補完応答のsubscriptionを任意とし、提示されたIDだけを検査する。account PUSHの購読は引き続き必須。購読情報なしの初回・継続ページを既存Smokeへ加え、保存checkpointを確認する。再試行期限は維持し、エラーを消すために受信cursorや待機トークを破棄しない。実環境での補完成功は配備後に別途記録する。

86c1ca8を本環境へ配備した後、同じCore snapshotから補完を再開し、3回のトーク取得と185イベントの受付経路が成功。補完待ち0 / 補完エラー0、health 200を確認した。過去履歴はbaselineと重複判定を通し、時刻を新着へ書き換えない。[配備時の計測](../operations/MINIMAL_BOT.md#2026-10-03の軽量ログ切替)。

## 14. 投稿なしの参加取得と配送待機Context（2026-10-03）

機密ストレージの直近ログをローカルだけで照合した。参加時刻から次の投稿まで約471秒の例があり、別の例は約15秒、退出は約7秒だった。旧ログには取得時刻がないため、これだけで後続投稿が受信を起こしたと断定しない。型とLINEJS 3.4.2のトーク取得を基に、投稿から独立した補助取得を追加した。[Adapterの周期と共通cursor](../../apps/line/docs/ADAPTER.md)。模擬PUSH補完とpollを重ねても同chatの実取得は1件、投稿なしでmember状態・通知Action・checkpointが保存されることを既存smokeで確認する。

本番ログにはSendBoundaryNotReachedによる再起動があった。ApiSchedulerの待機RPCを別RPCの完了Contextから起動すると送信者のAsyncLocalStorageが失われる経路を実SDKの模擬transportで再現した。enqueue時のSendAttemptを保持し、pumpでそのScopeへ戻して実行する修正ccbfcc3を行った。修正前は通信後例外をqueuedへ誤分類し、修正後はsending記録とunknown確定を確認した。motionのOOMによる再起動と混同しない。[メモリ実験](../../experiments/motion-memory/docs/MEMORY.md)。

## 15. 参加トーク一覧の実API差（2026-10-04）

6537914の起動直後、getJoinedSquareChatsがNOT_IMPLEMENTEDで失敗し、補助取得の対象が0だった。PUSHと保存復元は継続していた。採用SDKのClient.fetchJoinedSquareChatsはfetchMyEvents({limit:200})からnotifiedCreateSquareChatMember.chatを抽出している。これを上限・継続ページ付きのjoinedChatPageとして共有し、受信checkpointを変更せず一覧だけを読む。通知設定済みトークの取得は一覧成功に依存させない。既存smokeでsnapshotの次ページにsyncToken・continuationTokenが渡ることと、一覧API失敗時の通知取得を確認する。実APIでの対象件数と補助取得は配備後に記録する。

6825fa2を配備し、一覧失敗0、定期取得12トーク・優先5トーク、起動約654秒で定期取得829周期を確認した。古い通知設定だけのトークも対象へ含めていたため、1トーク相当のNOT_FOUNDがattempts 1〜9でbackoffした。他の取得・PUSHは継続し、API制限0、再起動0。未参加・削除済み等のどれかはエラーcodeだけでは確定しない。利用者からも未参加トークの可能性を指摘された。

最初のログ同期27行は、既存payload8件への追記と新payload1件。manifest9件のSHA256・gzip全行・件数を遠隔blobと照合した。messages 21 / names 3 / member-events 3。member-eventsはsource=poll、元のjoin/leave種別・取得時刻を保存していた。新しい退出の時刻差12秒の例と、約2時間以上前の参加退出の回収が含まれる。短時間・少数例なので3秒の通知保証やPUSHだけでの網羅性とは扱わない。

## 16. 未参加の旧設定と定期取得（2026-10-04）

Receiver.runPollingは、一覧を全ページ取得できた場合だけ、その一覧を定期取得対象にする。旧通知設定は削除せず、一覧にない件数をunlistedPriorityChatsへ出す。初回の一覧障害時は設定由来の取得を維持し、更新失敗時も前回一覧を維持する。参加一覧を使うことで無効な設定先への定期要求を抑えるが、NOT_FOUNDのすべてが未参加を意味するとは判断しない。PUSH補完は従来どおり別経路で残す。

既存Smokeで、一覧障害時も投稿なしの通知取得が動くことと、正常な一覧にない設定先へ要求せず設定行は残すことを確認した。実環境の一覧件数・除外件数・エラーは反映後に記録する。
