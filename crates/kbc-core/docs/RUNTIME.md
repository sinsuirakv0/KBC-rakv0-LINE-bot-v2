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

txtと同梱検索を受付でAction化する。検索の走査は不変snapshotからSQLite lockの外で行う。Nativeの4受付・実処理1件のblocking workerへ渡し、Nodeを検索・fsyncの待機から分けた。外部APIをtransaction内で待たず、送信完了にも依存しない。一般的な永続Job Workerはまだ作らず、さらに重い処理では未処理本文の保存を設計する。[Commandの仕様と関数](../src/commands/docs/COMMANDS.md)を参照。

## 保存と上限

- `events`: 処理済みEventのIDと受信時刻。既存の`payload`列は空文字にする。IDはトークIDとmessage IDの組で、受信元が異なっても同一になる。現在の軽いCommandは受付transaction内でActionへ変換済みのため本文を保持する必要がない。一般Workerの実装時は未処理本文の保存を別途設計する。
- `actions`: 宛先・本文・期限・状態。`queued → claimed → sending → sent / failed / unknown`。同じ宛先の`claimed`と`sending`は同時に一つ、他の宛先はAdapterの2配送Workerで独立する。未来の通知期限が即時返信を塞がないようdue順で選ぶ。通信前の再待機は最大600秒後、現Adapterは1秒後。再待機後もdue順を使う。
- `checkpoints`: stream別のopaque JSON。全体の再開位置には補完待ちchatも含み、途中再起動で補完指示を失わない。
- 重複IDの既定上限は131,072件、`CoreConfig.maxRetainedEvents`で8,192〜524,288へ設定可能。未解決Action（queued / claimed / sending / unknown）は2,048件。完了履歴（sent / 確定failed）は別枠で最新2,048件まで、本文を空にする。Batch 100件・256KiB、入力本文32KiB、checkpoint 64KiB。SQLiteのpage数上限16,384（通常4KiB/pageで64MiB）。journalの一時使用は別に発生する。
- 重複IDと完了履歴は48時間後、受付時に最大毎分1回掃除する。未解決Actionとその重複IDは容量確保のために消さない。cleanupの関連検索には`actions(event_id,status)`索引を使う。容量不足・保存失敗ではBatch全体をrollbackし、checkpointを確定せず停止する。

`stats`に重複ID数と上限、queued / claimed / sending / unknown / failed / 完了履歴と期限内Sessionの件数を出す。上限は多数OCの継続運用を検証済みという意味ではない。長いIDやcheckpoint数では件数上限より先にbyte上限へ到達し得る。保持期間より古い既処理IDを再取得する場合の重複防止は未保証。LINEの再取得範囲と合わせて運用観測で決める。

初回はAdapterが`baselineBeforeMs`を渡し、起動前の履歴から返信を作らない。通常再起動は保存されたorigin・checkpointを再利用する。初回baselineの時刻はLINE側とローカル時計の差の影響があり、実運用で確認する。

## Bridge

`kbc-protocol`のRust型から`apps/line/src/protocol/generated/`を生成する。本人・返信先・送信ID・画像URLを追加したLINE専用Protocol v3。NativeとAdapterを同時に更新し、旧Adapterと混在させない。`kbc-node`は変換とRuntime呼出だけを行う。

JSからNativeへの設定・Batch・結果は型付きDTOをJSON文字列化して渡す。N-APIのserde Value変換では整数の時刻がf64として入ってi64の復元に失敗したため、この小さな境界では整数表現を保つ。出力はplain DTO。大きな画像・SDKオブジェクトをこの経路へ渡さない。

実受信は`submitBatchAsync`でblocking workerの保存完了を待つ。同期`submitBatch`はオフライン検証用に残す。結果登録等は短い同期transaction、期限待ちの`nextAction`はTokio側。画像取得は`prepareImage`で共通Rust Clientへ渡し、PNG本体だけを最大2MiBのBufferでAdapterへ返す。大容量データの一般Protocolは作らない。

[今回の決定・検証](../../../docs/decisions/FOUNDATION_RECOVERY_V1.md)に容量再現、compact IDの実測、既存DBの扱いを残す。
