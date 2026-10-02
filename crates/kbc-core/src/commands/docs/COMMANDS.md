# LINE Commandの最初の移植

2026-10-02。状態: 実装・Windows Nativeのオフライン検証済み。実LINEへの配備・表示・画像送信は未確認。

## 仕様

| 入力 | 動作 |
| --- | --- |
| `o.ping` | txtの `pong!` を返す |
| `o.help` / `o.help ut` / `o.ut help` | 実装済み一覧 / コマンド別txtの案内 |
| `o.ut` / `o.tut` / `o.st` | JDBの検索ページURL |
| `o.ut 0` / `o.tut わんこ` / `o.st N000-000` | ID・名前・別名で検索し、詳細URLまたは候補を返す |
| `o.st 3000-000` | 日本編等の数値IDからもJDBリンクを解決 |
| `o.ut 検索語 -f` | 元の表記へ検索。`-force` も可 |
| `o.ut 0 origin c` / `o.tut 0 origin` | 詳細案内とPNG画像。utの形態はf/c/s/u、既定f |
| `o.test-notify 5` | 既存の自律通知確認 |

`unit / enemy / stage` は `ut / tut / st` の別名。prefixは小文字の `o.`、コマンド名はASCII大文字小文字を区別しない。検索語は16語・512byteまで。通常検索は全角英数字・ひらがな/カタカナ・長音・波線を旧LINEと同様に正規化し、空白区切りの語をANDで照合する。`-f`は表記を変えない。実データを起動時に正規化して保持し、問い合わせごとの再正規化やHTTP取得はしない。

通常検索の1〜3件は詳細を一つの返信へまとめる。4件以上は8件ずつ。最大512候補を保持し、それより多い場合は総件数と絞り込みの案内を出す。originは1候補なら直送、複数なら本人の選択を待つ。旧Botの「先頭の候補を勝手に画像化」から変更した。Discord固有のfile・motion等は旧LINEにないため今回の移植対象に含めない。

表示は番号、名前、空行、URLのプレーンテキスト。罫線・コードブロック・桁そろえを使わない。出力は改行優先で1,500 UTF-16単位に分割し、1入力のActionは最大8件。通常の検索・候補ページは1返信、originは案内1返信と画像1件（LINE RPCとOBS uploadの2通信）。

## 番号リプライ

候補ページへ本人がリプライした場合だけ反応する。1〜8で詳細を選び、9で次、0で前、終了 / 取消 / cancelで終了。次 / 前の文字も使える。通常の数字メッセージ、別の人、別トーク、過去のページへのリプライは無視する。

同じ人・トークにつき一つ、全体128 Session、候補512件、送信成功から10分。未送信の受付も生成から10分を過ぎれば失効する。新しい検索で同じ人・トークの受付を入れ替える。他の人・トークの受付は維持する。詳細選択後は終了する。ページ移動は新しいメッセージを送り、その送信IDだけを次の受付先にする。操作対象のmessage IDを知るための追加LINE APIは呼ばない。

SQLiteのSessionにはowner、chat、送信Action ID、実際の送信message ID、検索条件、候補行番号、ページ、期限、snapshot指紋を保存する。受信ID・Action・Session変更・checkpointは同じtransactionで確定する。送信IDは `completeAction` の成功結果で結び付ける。結果不明は自動再投稿せず、照合した `resolveAction` にmessage IDを渡した場合だけ復旧できる。保存ファイルが残る再起動では受付を保持し、データ変更・期限切れでは失効する。

期限は受付・起動・選択時に判定する。上限到達は検索が混んでいる旨を返信し、既存の別Sessionを押し出さない。

## 編集の代替とOCの管理者削除

利用者の追加指定に従い、管理者の `square.destroyMessage` を使う。ページ更新・詳細選択・終了・同じ人の再検索では、新しい返信の送信IDと受付を確定してから古いBot候補のDeleteMessage Actionを配送する。新規返信の送信失敗では古い候補を削除しない。既存ページを再掲するだけの無効入力でも削除しない。

送信した候補には10分後の削除Actionを同じCoreの期限管理へ登録する。操作で入れ替わった場合はそのActionを前倒しし、同じchat / message IDへ二重の削除を発行しない。期限終了だけを知らせる新規投稿は行わない。削除時のLINE APIは1回で、常時巡回・追加照会・独立Timerは作らない。削除の失敗・結果不明は返信とは別に保持し、確定済みの新promptと番号受付を巻き戻さない。結果不明の削除も自動再試行しない。

削除対象はCoreが送信結果で取得したBotの候補IDだけ。ユーザーの入力や通常の検索結果URLは削除しない。削除にも2,048 Actionの共通上限を適用し、満杯時は新たな清掃を追加しない。通常は送信完了で空いた1枠を期限清掃が引き継ぎ、古い清掃は前倒しで再利用する。OCの実際の管理者権限・削除成功は実LINEで確認する。

## 関数の関係

| 関数 | 働き・呼出関係 |
| --- | --- |
| `ContentCatalog::load / command_help` | txt検査・不変catalog・実装済み一覧。`prepare`から利用 |
| `SearchCatalog::load / search` | 同梱snapshot検査・正規化・候補の走査。SQLite lockの外で実行 |
| `prepare` | prefix・別名・help・検索・通知を共通解析し、Text / Search / IgnoreのPlanを返す |
| `sessions::apply` | `Runtime::submit_batch`のtransaction内でPlanと本人のリプライをAction本文・Session変更へ変換 |
| `SearchCatalog::page / detail / image_url` | LINEの候補・URL・旧LINEの画像pathを組み立てる |
| `split_responses` | 文字を壊さず改行優先で出力を分割。画像Actionはそのまま渡す |
| `NativeCore::submit_batch_async` | 4受付・実処理1件の有限枠でRust blocking workerへ渡す。受信器はcommitをawait |
| `Runtime::finish_action / enqueue_cleanup` | 送信結果・Sessionのprompt・旧promptの削除と期限清掃を同じtransactionで保存。prompt送信成功でID欠落は拒否 |
| `ImageService::download / Runtime::prepare_image` | 共有ClientでPNGを最大2件、10秒、各2MiBまで取得。失敗は未通信Actionを元URL付き通常返信へ変換 |
| `deliverAction` | Core取得の画像をBlobへ変換。LINEJSのIMAGE送信後にOBS upload。DeleteMessageは管理者削除へ変換。各操作の例外は独立したunknownを保存 |

snapshotは4MiB、各種類30,000件、1項目の別名16件まで。現在の9,159件を常駐させる。検索Cache、コマンド別HTTP Client、独立したQueue、常時更新Timerは追加しない。外部画像はHTTPSのJDBとPONOSのみ、redirectは受け付けない。

## 検証と限界

送信成功後だけ古い一覧の清掃を前倒しすること、期限清掃、Coreが新しいpromptを確定した後の管理者削除の失敗でも番号操作が続くことを同じSmokeで確認した。管理者削除の実通信は模擬し、実OCでの削除権限と見え方は未確認。

`npm run smoke:commands` の4経路で、txt追加とBOM/CRLF、実データのリンク、本人・返信先・トーク、再起動・ページ更新・期限、画像取得前のfallback、OBS失敗のunknownを検証。LINE/外部HTTP通信は模擬または拒否し、実送信は行っていない。既存Smokeは非同期受付後の最終checkpointまで待つよう修正し、基盤の回帰も確認する。

Windows / Node 24.15.0 / Native releaseで、snapshot読込込みの起動154.951ms、RSS増分8,650,752byte。`o.st ネコ` の検索・保存20回はp50 11.766ms / p95 12.903ms。100入力の一括受付105.714ms、その間のNode 5ms Timerは7回動いた。応答配送は含まない。ビルド等が動くローカル測定であり、0.2コアのコンテナ性能や実LINE応答時間は未評価。

一般的な重いCommandの永続Job Worker、データの自動更新、認証・Coreのコンテナ消失時の復元、LINE thread、長期OCログは後続。軽い同梱検索は受付でAction化できるため、今回独立した永続Job Queueは増やさない。
