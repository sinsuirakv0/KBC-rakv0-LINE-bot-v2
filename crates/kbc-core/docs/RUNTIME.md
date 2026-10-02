# 最小Rust Runtime

作成日: 2026-10-02。状態: 最小実装・Windows Nativeでオフライン検証済み。実LINEと長期負荷は未検証。

## 責務と関数

| 関数 | 働き・関係 |
| --- | --- |
| `Runtime::open` | SQLiteを開き、アカウント所有者を照合。前回の`claimed`を再待機、`sending`を`unknown`へ変える。旧試作DBの処理済み本文を空にする |
| `submit_batch` | Protocol・件数・byteを検査。EventのID重複排除、疎通Commandの解析、Action生成、checkpoint更新を同じtransactionでcommit |
| `command_responses` | `o.ping`と確認用の期限通知を解析。LINE通信や外部HTTPを行わない |
| `checkpoint` | Adapterのstream別再開位置を返す。保存内容の意味はAdapterが所有 |
| `next_action` | 実行可能なActionを取得して`claimed`を永続化。空ならNotify、将来期限ならTimerで待つ |
| `mark_sending` / `retry_action` | 実transportの直前に`claimed → sending` / 通信前に失敗した`claimed`だけを期限付きで再待機へ戻す |
| `complete_action` | 送信結果を別経路で保存し、空いた宛先の配送を起こす |
| `resolve_action` | 運用者が照合した`unknown`を`sent`または確定`failed`へ明示的に解決する。自動再投稿はしない |
| `stats` / `shutdown` | 容量・配送状態を観測 / 待機を解除して停止 |

この段階のCommandは軽い2種類なので、解析とAction生成を受信transaction内で行う。一般的なCommand Workerはまだ作らない。API応答をtransaction内で待たず、送信完了にも依存しない。重いCommandの移植時には永続Inboxと有限Workerへ分割する。

## 保存と上限

- `events`: 処理済みEventのIDと受信時刻。既存の`payload`列は空文字にする。IDはトークIDとmessage IDの組で、受信元が異なっても同一になる。現在の軽いCommandは受付transaction内でActionへ変換済みのため本文を保持する必要がない。一般Workerの実装時は未処理本文の保存を別途設計する。
- `actions`: 宛先・本文・期限・状態。`queued → claimed → sending → sent / failed / unknown`。同じ宛先の`claimed`と`sending`は同時に一つ、他の宛先はAdapterの2配送Workerで独立する。未来の通知期限が即時返信を塞がないようdue順で選ぶ。通信前の再待機は最大600秒後、現Adapterは1秒後。再待機後もdue順を使う。
- `checkpoints`: stream別のopaque JSON。全体の再開位置には補完待ちchatも含み、途中再起動で補完指示を失わない。
- 重複IDの既定上限は131,072件、`CoreConfig.maxRetainedEvents`で8,192〜524,288へ設定可能。未解決Action（queued / claimed / sending / unknown）は2,048件。完了履歴（sent / 確定failed）は別枠で最新2,048件まで、本文を空にする。Batch 100件・256KiB、入力本文32KiB、checkpoint 64KiB。SQLiteのpage数上限16,384（通常4KiB/pageで64MiB）。journalの一時使用は別に発生する。
- 重複IDと完了履歴は48時間後、受付時に最大毎分1回掃除する。未解決Actionとその重複IDは容量確保のために消さない。cleanupの関連検索には`actions(event_id,status)`索引を使う。容量不足・保存失敗ではBatch全体をrollbackし、checkpointを確定せず停止する。

`stats`に重複ID数と上限、queued / claimed / sending / unknown / failed / 完了履歴の件数を出す。上限は多数OCの継続運用を検証済みという意味ではない。長いIDやcheckpoint数では件数上限より先にbyte上限へ到達し得る。保持期間より古い既処理IDを再取得する場合の重複防止は未保証。LINEの再取得範囲と合わせて運用観測で決める。

初回はAdapterが`baselineBeforeMs`を渡し、起動前の履歴から返信を作らない。通常再起動は保存されたorigin・checkpointを再利用する。初回baselineの時刻はLINE側とローカル時計の差の影響があり、実運用で確認する。

## Bridge

`kbc-protocol`のRust型から`apps/line/src/protocol/generated/`を生成する。送信状態の契約変更によりLINE専用Protocol v2。NativeとAdapterを同時に更新し、旧v1 Adapterと混在させない。`kbc-node`は変換とRuntime呼出だけを行う。

JSからNativeへの設定・Batch・結果は型付きDTOをJSON文字列化して渡す。N-APIのserde Value変換では整数の時刻がf64として入ってi64の復元に失敗したため、この小さな境界では整数表現を保つ。出力はplain DTO。大きな画像・SDKオブジェクトをこの経路へ渡さない。

受付・保存・結果登録は同期Native呼出で、Nodeのイベントループ上で短いSQLite transactionを行う。重い処理の非同期化は受付時間の実測後に判断する。期限待ちの`nextAction`はTokio側で待つ。

[今回の決定・検証](../../../docs/decisions/FOUNDATION_RECOVERY_V1.md)に容量再現、compact IDの実測、既存DBの扱いを残す。
