# gatya・sale・itemの移植

2026-10-07。Discord v2のRust実装をLINE Coreへ移植。TypeScript AdapterとProtocol v18は変更しない。

## 入力と表示

`!` が既定、`o.` も同じ入口。公開helpは `!help gatya / sale / item`。

| コマンド | 対応する入力 |
| --- | --- |
| gatya | 開催中・予定一覧、R/E/Nによる種類指定、ガチャID、sシリーズID、シリーズ名検索、ID・シリーズIDにj/json・r/raw |
| sale | 開催中・予定一覧、イベント・ミッションID、名前検索、IDにj/json・r/raw |
| item | 配布一覧、giftType、該当なしならeventID、IDにj/json・r/raw |

gatyaの詳細はノーマル・レア・激レア・超激レア・伝説レアの順。0の区分は省略する。Discord版はsuperRareを読み込んでいたがformat_ratesの列挙に含めていなかったため、両Botで修正した。数値は公開データとDiscord版に合わせ、独自の換算を行わない。シリーズ一覧・名前検索は対応表全体を対象とし、シリーズJSON/rawはスケジュールJSONにあるガチャのblockだけを対象にする。

期間・必要版・常設・時間帯・確定等、saleの代表ステージとミッション、itemのギフト詳細・HTML改行もDiscord版を参照する。コードブロックを付けず最大32件、各1,500 UTF-16単位の通常メッセージへ分割する。JSONからrawを除外し、raw表示はタブを空白へ置き換える。rawがない場合はその旨を表示する。

利用者指定で、saleの名前検索が9件以下なら一覧へのリプライで番号を選択する。同じ送信者・トーク・一覧のmessage IDに限定し、「終了」で取消す。候補はID順。9件超・送信者不明・1メッセージに収まらない場合はID指定用の一覧を返す。選択後はIDで最新データを取得するため、検索と選択の間に公開データが更新されれば詳細は新しい内容となる。

## 関数と処理経路

```text
commands::prepare → sessions::apply → PrepareMedia(EventData)
→ media_loop → prepare_event_data → Source → 各run / Formatter
→ finish_event_data → 既存Outbox → 通常配送
sale候補送信成功 → sessions.prompt確定 → リプライ → select
→ PrepareMedia(sale ID) → 同じ取得・配送経路
```

| 関数・型 | 働きと依存関係 |
| --- | --- |
| Request / commands::prepare | コマンドと引数を永続化。16引数・512byteまで。ownerはsessions::applyでEventから設定 |
| Source | 最新JSONのheader日時を検証し、必要な名称・対応表をMetadataから取得 |
| skd::model / metadata / labels | skdと3コマンドで日時・モデル・名前・代表ステージ等を共用。rawは差分の比較対象外 |
| gatya / sale / item::run | 引数解釈、検索、Formatter選択。HTTP失敗はstderrへ詳細、利用者には取得失敗文 |
| push_message / commands::split_text | 通常返信と同じUTF-16・改行境界で分割し、32件超を失敗へ変換 |
| prepare_event_data / finish_event_data | 受付transactionの外でHTTPを実行し、返信と候補Sessionを結果transactionへ保存 |
| select / sessions::apply | 所有者・一覧・期限を既存Sessionで確認。IDの取得jobへ変換し、候補を終了 |

定型文は `content/messages/event.txt`、共通ラベルは既存 `skd.txt` 等。文面schema・help・indexも同時に更新する。

## 上限・停止・復旧

既存AssetServiceのClient・HTTP同時2枠・15秒・1レスポンス4MiBを共有する。任意のシリーズ名称は404だけを空として許容し、その他の通信失敗は失敗応答へ変換する。取得結果は処理後に破棄する。Cache・監視・専用Queueは追加しない。

既存準備Workerは1個、未解決準備・素材は合計8件。待機10分・実行10分とOutboxの予約容量を共有する。motion等の先行準備が長いと取得開始も遅れる。取得中の取消はHTTP Futureをdropし、未通信jobの復旧契約を使う。通常配送と受信は準備完了を待たない。

Sessionは全体128件、同じ人・トークの候補を置き換える。保存するのは最大9個のIDで、送信後30秒まで有効。送信成功時だけpromptと期限を確定し、unknownは自動再送しない。再起動後も未失効の候補を復元する。古い準備結果で後から作った検索候補を置き換えない。候補を開始できなくてもID一覧を保持する。

## 確認

2026-10-07、Core通常10件、公開データ単発確認、Native/TypeScript build、workspace/all-targetsのClippy警告拒否、smoke:commands・skd・messagesが通過した。

公開データでは3コマンドの一覧・ID詳細・JSON・rawの12通りに加え、gatyaのRシリーズ指定・名前検索・シリーズJSONとsaleの名前検索・選択開始を確認。恒久の最小回帰は激レア表示と、候補の送信成功・30秒期限・所有者・再起動の2件。公開取得は通常テストではignoreする。

NativeのsubmitBatchAsyncから共通Worker・返信ActionまでLINE通信0件で確認。準備待ちのsaleより先にpingを配送でき、sale候補のリプライ選択から詳細を生成し、!gatya・o.itemも応答を生成した。実OC送信と本番配備は今回実施していない。LINEJS最新公開版は3.4.2を再確認し、依存は維持した。
