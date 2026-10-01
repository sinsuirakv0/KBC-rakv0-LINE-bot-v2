# 最小Rust Runtime

作成日: 2026-10-02。状態: 最小実装・Windows Nativeでオフライン検証済み。実LINEと長期負荷は未検証。

## 責務と関数

| 関数 | 働き・関係 |
| --- | --- |
| `Runtime::open` | SQLiteを開き、アカウント所有者を照合。前回の` sending `を`unknown`へ変える |
| `submit_batch` | Protocol・件数・byteを検査。EventのID重複排除、疎通Commandの解析、Action生成、checkpoint更新を同じtransactionでcommit |
| `command_responses` | `o.ping`と確認用の期限通知を解析。LINE通信や外部HTTPを行わない |
| `checkpoint` | Adapterのstream別再開位置を返す。保存内容の意味はAdapterが所有 |
| `next_action` | 実行可能なActionを取得して` sending `を永続化。空ならNotify、将来期限ならTimerで待つ |
| `complete_action` | 送信結果を別経路で保存し、空いた宛先の配送を起こす |
| `stats` / `shutdown` | 容量・配送状態を観測 / 待機を解除して停止 |

この段階のCommandは軽い2種類なので、解析とAction生成を受信transaction内で行う。一般的なCommand Workerはまだ作らない。API応答をtransaction内で待たず、送信完了にも依存しない。重いCommandの移植時には永続Inboxと有限Workerへ分割する。

## 保存と上限

- `events`: 正規化した本文付きEventと受信時刻。IDはトークIDとmessage IDの組で、受信元が異なっても同一になる。
- `actions`: 宛先・本文・期限・状態。`queued → sending → sent / failed / unknown`。同じ宛先は同時に一つ、他の宛先はAdapterの2配送Workerで独立する。未来の通知期限が即時返信を塞がないようdue順で選ぶ。
- `checkpoints`: stream別のopaque JSON。全体の再開位置には補完待ちchatも含み、途中再起動で補完指示を失わない。
- 件数はEvent 8,192、Action 2,048、Batch 100件・256KiB、本文32KiB、checkpoint 8KiB。SQLiteのpage数上限16,384（通常4KiB/pageで64MiB）。journalの一時使用は別に発生する。
- 正常配送済みのActionと対応Eventは48時間後、次の受付で削除する。待機・送信中・失敗・結果不明を容量確保のために消さない。容量不足・保存失敗ではBatch全体をrollbackし、checkpointを確定せず停止する。

この件数・保持時間は試験用の初期値。多数OCの継続運用を検証済みという意味ではない。容量到達、削除負荷、Native受付にかかる時間を測り、保持と運用手順を見直す。処理済みと結果不明を無期限・無容量で保持する仕組みにはしない。

初回はAdapterが`baselineBeforeMs`を渡し、起動前の履歴から返信を作らない。通常再起動は保存されたorigin・checkpointを再利用する。初回baselineの時刻はLINE側とローカル時計の差の影響があり、実運用で確認する。

## Bridge

`kbc-protocol`のRust型から`apps/line/src/protocol/generated/`を生成する。LINE専用Protocol v1。`kbc-node`は変換とRuntime呼出だけを行う。

JSからNativeへの設定・Batch・結果は型付きDTOをJSON文字列化して渡す。N-APIのserde Value変換では整数の時刻がf64として入ってi64の復元に失敗したため、この小さな境界では整数表現を保つ。出力はplain DTO。大きな画像・SDKオブジェクトをこの経路へ渡さない。

受付・保存・結果登録は同期Native呼出で、Nodeのイベントループ上で短いSQLite transactionを行う。重い処理の非同期化は受付時間の実測後に判断する。期限待ちの`nextAction`はTokio側で待つ。
