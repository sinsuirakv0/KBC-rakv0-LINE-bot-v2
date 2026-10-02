# コマンド移植前のGPTレビューと照合結果

記録日: 2026-10-02。対象コード: `6027cfca04bc21217ca7d6eb57fce7943b8b0fd9`。
状態: 外部レビューとローカル照合が完了。以下の修正は未実装。

## 実施範囲

Chromeの通常ChatGPTで、GitHubプラグイン付きの読み取り専用レビューが完了した。画面には「15m 56s考えました」と回答完了が表示された。Workへの切替、思考設定の変更、生成の停止、思考中の追加依頼は行っていない。サイドバー側の生成エラーの記録は[依頼資料](../engineering/FOUNDATION_REVIEW.md)に残す。

GPTの回答を現在のCore / Adapterと、lockで固定されたLINEJS 3.4.2配布物に照合した。GPTによるコード閲覧の申告と、この作業でのソース確認・実験結果を区別する。実LINEへのログイン・送信・制限試験、認証値・OCログの読取、非公開ストレージの変更は今回行っていない。

結論は、Rust Core / 薄いTS Adapter / PUSHの分離を維持し、次の4点を先に扱うこと。原子的な受付・checkpoint、期限による自律配送、同じcursorの直列取得、有限API枠を作り直す根拠は見つかっていない。一方、現状を多OCの長期実運用が確認済みの基盤とは扱わない。

## 先に扱う4点

| 対象 | 確認した経路と影響 | 最小修正の方向（提案） |
| --- | --- | --- |
| 未送信Actionの結果不明化 | Core `next_action()`がJSへ返す前に`sending`を保存する。SDKはその後にreqseqを保存し、API枠・通信へ進む。保存失敗や枠の拒否など、送信前の失敗もAdapterで一律`unknown`となり、返信が再配送されない | 取り出し済みと通信開始の状態を区別し、確実に通信前と判定できる失敗だけ再待機へ戻す。通信開始を記録する境界と再起動契約を揃える |
| AuthStorage障害の連鎖 | `save()`は失敗したPromiseを次の保存の前提にするため、以後の保存処理が実行されない。`.auth`失敗はabortするが、reqseq保存失敗は配送側で`unknown`として扱われ、受信・healthが継続しながら以後の返信が失敗し得る | 認証・reqseq・refreshTokenの保存障害を共通の致命的障害へ伝えるか、整合性を保った限定復旧にする。失敗を握りつぶして継続しない |
| 補完失敗による全体停止 | `completePending()`は1トークでも失敗するとthrowし、次のsessionもPUSH初期化前に同じ補完を再実行する。権限喪失等で失敗が続くと、そのトークがアカウント全体の新着取得を塞ぐ | トーク別の有限な再試行・失敗記録を永続化し、未完了の補完を残して他トークの受信を進める。Core保存失敗は引き続き全体停止 |
| 保存上限と長期利用 | Event 8,192件・Action 2,048件は待機中だけでなく保持中の全行を数える。本文メッセージと送信済みActionを48時間残すため、通常利用でも上限に達する。`unknown`・`failed`には解決経路がなく、容量を占有し続ける | 有限な上限と保持方針を想定負荷で決め、使用量を観測する。重複判定用の記録と本文の保持を分ける案、結果不明の明示的な解決・退避を検討する |

参照関数と呼出関係:

- [Core](../../crates/kbc-core/src/lib.rs): `open()`で`sending → unknown`、`submit_batch()`で削除・件数判定・原子的保存、`next_action()`で取り出しと`sending`保存、`complete_action()`で結果保存。
- [配送・終了](../../apps/line/src/main.ts): `deliver()`からSDK `square.sendMessage()`を呼び、例外を一律`unknown`へ変換する。token更新の`authTask`だけは保存失敗でabortする。
- [認証保存](../../apps/line/src/adapter/storage.ts): `set()` / `delete()` / `clear()`が`save()`へ入り、直列Promiseからtmp書込・fsync・renameへ進む。
- [受信](../../apps/line/src/adapter/receiver.ts): `session()`は保存済み`pendingChats`を`completePending()` / `drainChat()`で補完してからPUSHを開始する。
- LINEJS配布物: `base/service/square/mod.ts:147`で送信前に`getReqseq("sq")`、`base/core/mod.ts:235`でreqseq保存を待つ。

送信状態の追加だけでDB保存と外部通信を原子的にはできない。通信開始直前に`sending`を保存しても、その直後の終了は結果不明になり得る。目的は、確定した未送信失敗を取り戻し、曖昧な外部操作を勝手に再投稿しないこと。HTTP 429を含む通信開始後の例外を、一律に自動再送へ変えない。

AuthStorageの保存列を止める挙動には、保存状態が不確かなまま認証・sequenceを進めない意味がある。GPTの「Promise列のrejectを回復させる」案だけを採用すると、この方針が変わる。先に、全体停止への伝達と保存前後のメモリ状態を含む復旧契約を決める。

補完の全体停止はコード上確認した経路。実LINEでどのerror codeが恒久失敗になるか、再取得期限がどれだけあるかは未確認。未完了トークを黙って削除して回避しない。

## 保存上限の最小再現

Node.js 24、既存のNative build、Node組込みSQLiteを使用。OSの一時ディレクトリに専用DBを作り、実アカウントを使わず実施した。

1. 合成ownerでCoreを開き、空Batchのcheckpointを保存する。
2. 当日完了した送信済みActionをSQLiteへ2,048行投入する。元Eventを含む2,048回の実コマンド試験ではなく、容量判定を切り出した合成fixture。
3. `queuedActions = 0`を確認し、異なる新Eventの`o.ping`をCoreへ投入する。
4. `StoreCapacity`を確認。新Eventはrollbackされ、checkpointも投入前の値を維持した。

観測結果: `completedActions=2048, queuedBefore=0, result=StoreCapacity, batchRolledBack=true, checkpointUnchanged=true`。通信なし、テストコードの追加なし。

48時間を均等に使う場合でも、Event 8,192件はアカウント全体で約171件/時、Action 2,048件は約43件/時に相当する。これはAPIの速度上限ではなく、保持件数が満杯になる目安。`o.test-notify`は1入力から2 Actionを作るため、入力件数だけで予算を見積もらない。

容量超過時のrollback・checkpoint維持は期待どおり。問題は、最小試験向けの保持設定と未解決Actionを、そのまま多OCの長期運用へ使えないこと。単に上限を無制限にする変更は採用しない。

## 追加の指摘と検証の扱い

| 項目 | 照合結果・次の小さい確認 |
| --- | --- |
| PUSH payloadのsubscription ID | SDK既定処理は`base/push/connManager.ts:440`でpayloadのIDを読み、取得に使う。現AdapterはPUSHをdirty通知として扱い、取得応答のIDを使う。この差は確認済みだが、実LINEでの影響は未確認。継続ページのIDを壊さずPUSHの新しいIDを扱う合成fixtureと実通信を確認する |
| 同時token refresh | SDK `tryRefreshToken()`には同時更新の合流がない。API枠2件でも、同じ旧refreshTokenから並行更新し得る。サーバーのtoken rotationへの影響は未確認。限定した共有Promiseで更新を合流できるか検討する |
| 終了時の保存完了 | `finally`は`.auth`の`authTask`を待つが、fire-and-forgetのnoopとSDKのrefreshToken / expire保存を全てjoinする契約がない。保存を発生させる仕事を止めて待ってから、Storage全体をflushする順序を検討する。20秒の終了上限は維持する |
| トーク補完のID検証 | `drainChat()`はsubscription IDを`Number()`で変換するだけ。正のsafe integerを確認してから保存する小さい修正候補 |
| SQLite cleanup | `actions(event_id,status)`の索引がなく、Event cleanupの関連検索が増える可能性がある。実行計画・時間は未計測。常時CPU問題の原因とはまだ断定しない |
| 429後の配送 | 共通cooldownがある一方、配送例外は`unknown`で止まる。通信前の失敗と、LINEが処理したか不明な応答を分ける。SDK内部の再要求を含め、LINEの意味を確認せず自動再送を増やさない |

PUSHのdirty通知集約に典型的な起床取りこぼしは、この静的レビューでは見つかっていない。有限枠の中でSDKの内部更新が再び枠を取得しない仕組み、PUSHのNode HTTP/2 transport維持、生成Thrift型によるトーク継続取得は維持する。これらは多OCの実証結果ではない。

## 移植を進める順序

1. 上記4点の状態・障害境界・保持方針を小さく修正し、重大回帰にだけ既存Smokeを補う。
2. GitHub認証復元とCoreの退避・復元を実装し、コンテナ交換時の永続性を確保する。少数OCでPUSH・送信・終了・refreshを確認する。利用者が後続観測へ回した手動の同時返信試験は再要求しない。
3. `help`の旧仕様調査と静的案内は先に準備できる。表示は実装済みCommandだけにする。
4. `intro`は旧コードのOS / Node / CPU / メモリ等の表示を確認し、必要な環境SnapshotをNodeからDTOで渡す。GPTが挙げたuptime・接続状態を、そのまま旧仕様として追加しない。
5. `id`は現DTOならchat MID / message IDに限定できるが、旧版の基本仕様を参照して移植範囲を決める。sender MIDやOC MIDは取得情報からProtocolへ追加し、chat MIDから推測しない。

重い外部HTTP・検索は有限Command Workerと共通HTTP / timeout / cancel、管理Commandは送信者・OC単位・権限・停止設定を先に揃える。受信がCommand用APIの待機列へ埋もれる負荷になった場合には、小さい優先度・受信枠を検討する。長期OCログの完成を静的`help`の条件にはしない。

今回の成果はレビュー・照合・容量再現と修正候補の記録。コード修正、新Botの配備、多OCの制限回避、取りこぼし防止の達成は含まない。
