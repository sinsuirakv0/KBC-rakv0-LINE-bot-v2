# 最小Rust Runtime

作成日: 2026-10-02。状態: 最小実装・Windows Nativeでオフライン検証済み。実LINEと長期負荷は未検証。

## 責務と関数

| 関数 | 働き・関係 |
| --- | --- |
| `Runtime::open` | SQLiteを開き、アカウント所有者を照合。前回の`claimed`を再待機、`sending`を`unknown`へ変える。旧試作DBの処理済み本文を空にする |
| `submit_batch` | Protocol・件数・byteを検査。検索をlock外で準備し、ID重複排除・Command / Session確定・Action生成・checkpointを同じtransactionでcommit |
| `commands::prepare / sessions::apply` | txt・検索・通知を解析。検索はlockの外、Action・Sessionは受付transactionで確定 |
| `checkpoint` | Adapterのstream別再開位置を返す。保存内容の意味はAdapterが所有 |
| `next_action` | 実行可能なActionを取得して`claimed`を永続化。空ならNotify、将来期限ならTimerで待つ |
| `mark_sending` / `retry_action` | 実transportの直前に`claimed → sending` / 通信前に失敗した`claimed`だけを期限付きで再待機へ戻す |
| `complete_action` | 送信結果・候補prompt・旧候補の削除と期限清掃を保存し、空いた宛先の配送を起こす |
| `resolve_action` | 運用者が照合した`unknown`を`sent`または確定`failed`へ明示的に解決する。自動再投稿はしない |
| `prepare_image` | shared HTTPでPNGを取得。未通信の取得失敗は元URL付き通常返信へ切り替える |
| `stats` / `shutdown` | 容量・配送状態を観測 / 待機を解除して停止 |

txtと同梱検索を受付でAction化する。検索の走査は不変snapshotからSQLite lockの外で行う。Nativeの4受付・実処理1件のblocking workerへ渡し、Nodeを検索・fsyncの待機から分けた。外部APIをtransaction内で待たず、送信完了にも依存しない。素材取得・描画はPrepareMedia Actionとして永続化し、通常配送と分けた共通Workerで処理する。詳細は [Media Worker](MEDIA.md) を参照。[Commandの仕様と関数](../src/commands/docs/COMMANDS.md)を参照。

## 保存と上限

- `events`: 処理済みEventのIDと受信時刻。既存の`payload`列は空文字にする。IDはトークIDとmessage IDの組で、受信元が異なっても同一になる。現在の軽いCommandは受付transaction内でActionへ変換済みのため本文を保持する必要がない。一般Workerの実装時は未処理本文の保存を別途設計する。
- `actions`: 宛先・本文・期限・状態。`queued → claimed → sending → sent / failed / unknown`。同じ宛先の`claimed`と`sending`は同時に一つ、他の宛先はAdapterの2配送Workerで独立する。未来の通知期限が即時返信を塞がないようdue順で選ぶ。通信前の再待機は最大600秒後、現Adapterは1秒後。再待機後もdue順を使う。
- `checkpoints`: stream別のopaque JSON。全体の再開位置には補完待ちchatも含み、途中再起動で補完指示を失わない。
- 重複IDの既定上限は131,072件、`CoreConfig.maxRetainedEvents`で8,192〜524,288へ設定可能。未解決Action（queued / preparing / claimed / querying / sending / unknown）は2,048件。完了履歴（sent / 確定failed）は別枠で最新2,048件まで、本文を空にする。Batch 100件・256KiB、入力本文32KiB、checkpoint 64KiB。SQLiteのpage数上限16,384（通常4KiB/pageで64MiB）。journalの一時使用は別に発生する。
- 重複IDと完了履歴は48時間後、受付時に最大毎分1回掃除する。未解決Actionとその重複IDは容量確保のために消さない。cleanupの関連検索には`actions(event_id,status)`索引を使う。容量不足・保存失敗ではBatch全体をrollbackし、checkpointを確定せず停止する。

`stats`に重複ID数と上限、queued / claimed / querying / sending / unknown / failed / 完了履歴と期限内Sessionの件数を出す。上限は多数OCの継続運用を検証済みという意味ではない。長いIDやcheckpoint数では件数上限より先にbyte上限へ到達し得る。保持期間より古い既処理IDを再取得する場合の重複防止は未保証。LINEの再取得範囲と合わせて運用観測で決める。

初回はAdapterが`baselineBeforeMs`を渡し、起動前の履歴から返信を作らない。通常再起動は保存されたorigin・checkpointを再利用する。初回baselineの時刻はLINE側とローカル時計の差の影響があり、実運用で確認する。

## Bridge

`kbc-protocol`のRust型から`apps/line/src/protocol/generated/`を生成する。本人・返信先・送信ID・画像URLとOC操作を追加したLINE専用Protocol v18。NativeとAdapterを同時に更新し、旧Adapterと混在させない。`kbc-node`は変換とRuntime呼出だけを行う。

JSからNativeへの設定・Batch・結果は型付きDTOをJSON文字列化して渡す。N-APIのserde Value変換では整数の時刻がf64として入ってi64の復元に失敗したため、この小さな境界では整数表現を保つ。出力はplain DTO。大きな画像・SDKオブジェクトをこの経路へ渡さない。

実受信は`submitBatchAsync`でblocking workerの保存完了を待つ。同期`submitBatch`はオフライン検証用に残す。結果登録等は短い同期transaction、期限待ちの`nextAction`はTokio側。メディアは`runMediaJobs`が共通Rust Client・Rendererで準備し、`prepareAttachment`で最大8MiBのBufferをAdapterへ返す。`prepareImage`は以前の未配送画像Action用の互換経路として残す。大容量データの一般Protocolは作らない。

[今回の決定・検証](../../../docs/decisions/FOUNDATION_RECOVERY_V1.md)に容量再現、compact IDの実測、既存DBの扱いを残す。

## OC管理の追加

初期OC対応ではMemberChanged、メッセージのOC・媒体・メンション情報、OcRequest / Resultを追加した。submit_batchの安価な候補判定からoc::ingestへ入り、照会結果と後続Actionをcomplete_actionのtransactionで確定する。[OC実装](../src/oc/docs/OC.md)を参照。

next_query_actionはcontext / member / chatsの読み取りだけをqueryingへ変える。1照会Workerが処理し、同じ宛先の通常claimed / sendingを塞がない。queryingは起動時queuedに戻す。membership / reportは通常配送のsending契約を使い、再起動・通信後失敗ではunknownとして保持する。通信前の確定失敗はclaimedからfailedにできる。OCの権限照会中でもpingを配送することをオフライン検証した。

eventsは本文を持たないが、未完了OC照会のcontinuationには元入力と必要なCommandPlanを一時保持する。完了時は既存契約どおりpayloadを空にし、unknownは照合まで保持する。OC設定・対話・短期状態も同じSQLiteとbyte上限を共有する。

## 遠隔退避

persistence_revisionはDB変更数、snapshot_databaseはlock下のVACUUM INTOで整合した一時DBを作る。遠隔復元時は期限済みの副作用Actionをunknownにし、二重送信を避けて運用照合を要求する。未来の通知と読み取りは維持する。[保存の契約と限界](../../../docs/operations/GITHUB_RECOVERY.md)。

Protocol v6は受信metadata・senderNameとログDTOを追加した。長期ログのpendingを共通transaction・Core snapshotに含める。`events`は引き続き重複ID用で本文を残さない。[ログの関数と上限](LOGS.md)。

Protocol v7はID照会用Members / JoinedChatsと参加退出のsource・取得時刻を追加する。照会は既存next_query_action、通常送信は非replyを既定とする。priority_chatsは設定DBから補助取得対象だけを返す。message_refsとoc_historyの追加列も共通DBのsnapshotに含める。[IDの関数と上限](../src/oc/docs/ID.md)、[独立取得と共通枠](../../../apps/line/docs/ADAPTER.md)。

## 検証OC操作（v8）

Protocol v8はInspect（読み取り）・Roles / Post / Delete（変更）とOcResultの送信IDを追加する。inspectをis_read・次の照会Actionを選ぶSQLへ同時に追加し、照会Workerへ渡す。許可OC tableは最大64件で共通DB・snapshotに含め、操作のcontinuationと結果は既存Outbox・oc_historyだけを使う。[関数・期限・許可の境界](../src/oc/docs/TEST_OC.md)。

Protocol v9はProfile変更要求を追加する。BOT管理者の!bot nameは既存Context / Member照会・変更配送・oc_historyを共有し、専用Queueや定期処理は持たない。[Botの表示名更新](../src/oc/docs/BOT.md)。

Protocol v10はOcResultのrawMemberNameを追加し、Bot名の完全一致判定へ未加工のLINEプロフィール名を使う。表示用DTOの短縮・改行整形とAPI確認を分ける。Bot固有の20文字・改行・制御文字制限は利用者指定で撤廃し、共通の入力・結果byte上限を維持する。

Protocol v11でReactionNotified / Reactions / OcReactionを追加したが、2026-10-04の利用者指定でページ操作への利用を廃止した。型は旧snapshotの復元用に維持し、旧Reactionsの結果でページを変更しない。sessions.pending_payloadはリプライによる変更先を保持し、complete_actionはSendMessage成功時だけ確定する。未知の送信結果を自動再送しない。Sessionのrevisionはsnapshotと表示件数を含め、旧8件表示は起動時に失効する。[ページ操作の関数・検証](../src/commands/docs/COMMANDS.md)。

Protocol v12はSendMessageに任意のMessageEmojiを追加した。旧保存Actionの未指定値はdefaultで復元し、新旧Native / Adapterは混在させない。現在のページ案内は絵文字装飾を付けず、型とAdapterは保存互換のため維持する。message_refsはスタンプとLINE絵文字の必要なIDだけを保持し、既存の件数・期間・DB byte上限とsnapshotを共有する。[IDの関数と上限](../src/oc/docs/ID.md)。

Protocol v14はOcRequest::Stickerに送信ID・version・任意STKOPTを追加した。既存Context・Mutation・GitHub snapshotを共有し、旧保存DTOは変更しない。[テストコマンドと配送](../src/oc/docs/TEST_STICKER.md)。

2026-10-04、利用者向け文面を[共通カタログ](MESSAGES.md)へ分離した。Protocol v14のSticker.textはCoreで設定した代替文をSTKTXTへ渡すための項目。旧保存Actionは従来の代替文へ復元し、NativeとAdapterを同時に更新する。

## 公開検索データの更新（2026-10-05）

Runtimeはsearch_path / search_liveと共通AssetServiceのClient・2枠だけを保持する。索引はprepare・検索Session処理・Media準備時だけ読み込み、処理後に破棄する。通常入力は読み込まない。searchDataLive=trueの本番では確認から120秒以上のデータを拒否し、取得失敗でBot全体を停止しない。Sessionは起動時に期限だけを削除し、データのrevisionは操作時に照合する。CoreConfigに任意searchDataLiveを追加したがEvent / ActionのProtocol v14は維持する。[取得の契約](../../../data/search/docs/SNAPSHOT.md)。

Protocol v16。2026-10-05、run_store_monitorsを追加し、Android/iOSのストア監視を既存Lifecycleへ接続した。検知版・トークごとの登録・Outboxを同じDBへ保存し、GitHub snapshotから復元する。[監視・容量・配送の契約](../src/store_update/docs/STORE_UPDATE.md)。

Protocol v17はSendMessageの任意threadRootId / threadContentsを追加する。親送信の成功と1秒以降の本文Actionの登録を結果transactionで保存する。unknownの本文は同じスレッドの後続を止め、確定failedならqueuedの後続を取消す。通常返信は継続する。run_store_monitorsへSKD確認を接続し、schedule_updates / schedule_targetsと予約を含むOutbox容量を既存SQLite・GitHub snapshotで扱う。[SKDの状態と上限](../src/skd/docs/SKD.md)。

Protocol v18はMessageMentionに任意additional（MID・UTF-16位置）を追加する。旧Actionの単独メンションはdefaultで復元し、同じNative / Adapter版を使う。!test mention-labelは共通TestInspect / Mutation・Outboxだけで一個の表示範囲または個別範囲を試す。新しいWorker・常時処理を作らない。[入力と実機確認](../src/oc/docs/TEST_MENTION.md)。

2026-10-06、BOT権限を常駐HashMapからSQLiteのbot_rolesへ移した。旧ファイルは初回取込のみで、設定解除を再起動で復活させない。bot_stopsは個別・全体停止、稼働秒数は現在のRuntime起動から計測する。stats_from_dbをstatusとstatsで共有し、DB lock内で再lockしない。Protocol v18は維持する。[仕様・上限・関数](../src/oc/docs/BOT.md)。

2026-10-07、gatya・sale・itemは受付をPrepareMediaへ変換し、外部HTTPを保存transactionの外で実行する。saleの候補は共通sessions tableへ最大9 IDを保存し、送信成功後30秒まで有効。event-v1のSession操作では検索索引を読み込まない。Protocol v18を維持する。[処理経路](../src/event_data/docs/COMMANDS.md)。

2026-10-08、pushの設定・トーク別走査位置・未走査範囲の重複印を同じSQLiteへ追加した。予約は未来期限の既存actions、名前解決は既存準備Worker、監視はrun_store_monitorsの寿命・HTTP枠へ接続する。push通知の取り出し時も停止を確認する。Protocol v18とAdapterは維持する。[入力・関数・上限・復旧](../src/push/docs/PUSH.md)。
