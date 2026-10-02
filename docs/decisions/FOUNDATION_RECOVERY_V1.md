# 基盤レビュー後の送信・保存・局所復旧

決定日: 2026-10-02。状態: 採用・実装済み・オフライン検証済み。実LINE・Northflankでの動作は未確認。
問題の対象版と根拠は[GPTレビューの照合結果](../research/FOUNDATION_REVIEW_RESULT.md)を参照。

## 送信の境界

`nextAction`は取り出した操作を`claimed`にし、同じ宛先の別操作を同時に取り出さない。SDKのreqseq保存、API枠と開始間隔、Thrift / LEGYの準備を通った後、共通`client.fetch`直前で`markSending`する。RPC名と送信AttemptをAsyncLocalStorageで伝え、内部refreshを送信開始として扱わない。採用配布物のLEGYは暗号化後に同じfetchへ進む。

通信前に失敗した`claimed`だけを1秒後の再待機へ戻す。保存障害なら同時に全体停止する。`sending`の例外・再起動は`unknown`として残し、HTTP 429も自動再送しない。再起動時の`claimed`は再待機へ戻す。Protocolをv2へ上げ、旧Adapterとの混在を拒否する。

DBとLINEは原子的に更新できない。開始記録直後の終了は依然として結果不明になり得る。この小さい境界で、確実に未通信の操作を失わず、曖昧な操作を重複投稿しない。`resolveAction`は照合済みのunknownをsent / 確定failedへ変える明示的な運用APIで、自動処理からは呼ばない。

## 保存障害と終了

AuthStorageは保存成功後だけメモリ状態を更新する。reqseq、`.auth`、refreshToken等のどの保存でも、失敗は`AuthStorageError`としてglobal abortへ伝える。以後の読書きは拒否する。保存列は最大64待機で、上限も同じ致命的障害として扱う。

Promise列だけ回復させる案は採用しない。認証・sequenceがディスクとずれたまま処理を続けず、最後に保存できたファイルから復元可否を確認して再開する。Receiverはkeepaliveを含む仕事を終了させ、その後mainがStorage全体をflushする。20秒の終了上限と、失敗時にもhealth等を閉じる後始末を維持する。

## トーク補完の局所復旧

account checkpointに未完了chatと失敗の試行数・再試行時刻を保存する。1回2トーク・各4ページまでとし、続きがあれば位置を保存して列の末尾へ譲る。トークのAPI失敗は2秒から15分まで間隔を増やし、他トークとaccount PUSHを継続する。成功したものだけ未完了から除く。

待機中の最短補完期限で自律的に起きる。補完だけのためにaccountの空取得は行わない。最大256トークとcheckpoint 64KiBの上限を持ち、満杯では位置を削除せず停止する。Core保存失敗・保存checkpoint破損は全体停止のまま。サーバーの恒久失敗codeや取得保持期限は実LINEで確認する。

## 容量と保持

現時点の軽いCommandは受付transaction内でAction化が完了する。処理済み本文を保存し続けず、EventはIDと受信時刻だけ残す。未配送・結果不明Actionは本文と位置を保持する。旧試作DBもこの前提で処理済みEventと完了Actionの本文だけ空にし、未解決Action・checkpoint・認証は消さない。長期OCログの保存・移行ではない。

重複IDは48時間・既定131,072件、設定範囲8,192〜524,288。未解決Actionは2,048件、完了履歴は別に最新2,048件まで・最大48時間とする。確定failedは完了履歴に含め、unknownは含めない。ID数と上限、配送状態、完了数をmetricsへ出す。SQLiteの64MiB page上限を維持し、件数よりbyte上限が先に来る場合も停止・rollbackする。

未解決ActionのIDは削除しない。関連索引を追加し、期限cleanupは受付時に最大毎分1回。保持期間後の再取得に無条件の重複防止を約束せず、一般Workerを追加する前には未処理本文の保存を再設計する。

## 検証

既存Smokeへ重大回帰だけを追加。通信なしで以下を確認した。

- claimedの再起動・再待機、sendingの再起動とunknown、送信開始後の再待機拒否、明示的なunknown解決。
- 実SDKのreqseq書込失敗で通信0件・queuedへ復帰・保存障害の全体通知。transport到達後の例外は1回の通信でunknownになる。
- 完了Action 2,048件・重複ID 9,000件でも新規受付可能。未解決Action 2,048件なら新規Commandはrollbackし、checkpointを維持する。
- トーク補完失敗中も継続ページと別トークの受信が進む。再起動で待機時刻を引き継ぎ、次の入力なしで再試行・復旧する。account空取得を増やさない。

容量Probe: Windows、Node 24、Native release、Node組込みSQLite。専用の一時DBへ54byteの合成IDを131,000件、空payloadで投入。DBは21,073,920byte（約20.1MiB）。空Batchの受付20回はp50 10.790ms / p95 12.449ms（同期保存を含む）。実環境の0.2コア性能・全BotのMemory・実LINEの安全負荷を示す測定ではない。再実行は同じschemaへ合成IDを投入し、`PRAGMA page_count × page_size`と`performance.now()`で受付を計測する。

Native build・型生成・TS build、整形、Clippy、Smokeを確認。Docker build、認証・CoreのGitHub復元、実LINEのPUSH subscription ID、同時refresh、少数OCの送信・長期負荷は未確認または未実装のまま。
