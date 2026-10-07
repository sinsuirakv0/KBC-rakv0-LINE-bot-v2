# にゃんこストア更新の検知・通知

2026-10-05。Discord Bot v2のstore_update.rsとnotification.rsのAndroid/iOS監視を移植。LINEのメンションは付けず、既存のOutboxと暗号化Core snapshotを使う。公開ストアの反映前は検知できない。

## 操作と権限

!pushsetting android / ios [on|off|status]。onは省略可、android,iosは両方を一括設定。!pushsetting statusは両OSの状態。通知先は実行したトーク単位で、o.も同じ。OC ADMIN（副官を含まない）またはBOT admin/modのみ変更・確認できる。!help pushsettingと引数なしの案内は公開する。

初回観測は基準値だけ保存し、通知しない。既に監視しているOSへの通知先追加では現在版を再投稿しない。監視停止中も保存基準を残し、再開時に進んだ版を比較する。初期値は登録なし。offは当該トーク・OSの未送信queuedだけ取消し、既に取得・通信したActionは従来の結果処理へ任せる。

## 取得と容量

登録先が存在するOSだけ、各OSにつき一つのLoopが取得完了後5秒待つ。AndroidとiOSは相互に待たず、素材と同じClient・HTTP同時2枠・各通信15秒・応答4MiBを共有する。LINE API枠は使わない。OC数によるストア取得数の増加はない。登録は両OS合計512トーク設定まで、Outboxは全機能共通2,048件まで。

- Android: 日本向けGoogle Play公開ページのds:5からRPC id/requestを読み、batchexecuteのversionNameを取得。RPC/schema失敗時は公開ページを読み直して一度だけ再試行。公式の安定APIではないためschema変更時は失敗として扱う。
- iOS: 日本向けApple Lookup（bundleId=jp.co.ponos.battlecats）のversionを取得。1件・正しいbundleIdを検査し、cache-busterを付ける。[Appleの案内](https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/iTuneSearchAPI/Searching.html)は概ね20回/分（変更される場合あり）。通常5秒周期は12回/分に相当し、候補の再確認が増える点も含めて制限の保証とはしない。

数値segmentが増えた候補を即時に再取得し、文字列まで一致した場合だけ通知する。巻戻り・不正形式・schema変更・確認不一致・通信失敗では基準を変えず、15/60/300秒へbackoff。成功で5秒に戻る。停止はHTTP・permit待ち・sleepをCancellationTokenで中断する。

観測時刻は初回・版変更・復旧または1分経過でだけDBへ保存し、5秒ごとのfsyncを避ける。statusの保存時刻と毎回の検査時刻は区別する。初回取得失敗も空基準とerrorとして保存し、未確認と表示する。

## 状態・配送とDiscordとの差

store_subscriptions(platform,chat)とstore_versions(platform,version,checked,error)は既存SQLite内へ保存する。設定・基準値は同じGitHub退避・復元の対象で、新しい同期Workerは追加しない。

Discordは全送信成功後に基準を進める。LINEでは検知版と対象ごとの通知Actionを同じtransactionで保存し、基準を進める。再起動後の同版の二重登録を防ぎ、配送は既存Outboxが独立して行う。容量不足・保存失敗ではtransactionをrollbackして以前の基準を維持し、次回取得で再確認する。本文は1,500 UTF-16単位まで、設定文面が超過しても基準を進めない。

この方式では確定failed/unknownでも版を巻き戻さず、同版通知を自動生成し直さない。unknownは既存の照合・解決手順を使う。未退避期間にコンテナが消失した場合の損失・重複リスクは既存GitHub snapshot契約と同じ。旧Discord/旧LINEの通知設定は自動導入せず、LINEの各トークで登録する。

通知本文はDiscord同様に新しい版・JST検知時刻・KBC差分URL・ストアURLを通常メッセージで送る。文面はcontent/messages/update.txtのkeyで変更する。全体helpのindexは手動更新する。

## 関数と境界

| 関数・型 | 働きと接続 |
| --- | --- |
| oc::store_setting::parse/execute | 入力を既存OC Context照会へ接続し、OC/BOT権限を検査してconfigureとreplyへ渡す |
| configure/status | 実行トークの登録・解除・保存状態を既存受付transactionで扱う。一括登録の上限は変更前に検査 |
| StoreVersionSource / GooglePlaySource / AppStoreSource | Discord由来の取得・schema検査・有限RPC再初期化。素材と共有するHTTPを使用 |
| run_store_monitors/store_loop/poll_store | Runtimeあたり一つの起動、2 OSの取消可能Loop。登録があるOSだけ照会・再確認・backoff |
| record_store / notification | 再度DB基準と登録先を照合し、版・全Actionを原子的に保存。既存enqueue_responsesとwakeを利用 |
| NativeCore.runStoreMonitors / main | 薄いN-APIとAdapter起動接続。監視停止は既存shutdown、配信は既存deliverAction |

Protocol v16のNativeとAdapterを同時に配備する。Event/Actionの新種は追加しない。

## 検証と未確認点

Source parserと版比較、初回無通知、未確認候補拒否、複数トークの1回登録、Android/iOS独立、再起動、解除、Outbox容量rollbackをRustで検査する。既存OC Smokeで権限・トーク単位・一括設定・保存復元・状態表示を検査する。公開ストアの単発取得はignore付きpublic_sourcesで明示的に実行し、定期テストからは接続しない。実LINE通知と多OCの配信遅延は配備後の確認事項。

2026-10-05、公開ストアへの単発取得でAndroid 15.7.1 / iOS 15.7.0を取得した。これは当日の観測値であり、今後の公開版やLINE通知成功の保証ではない。Source parser・保存/通知原子性・Outbox容量のRust検証は4件通過した。

build / TypeScript check / 全workspace・all-targetsのClippy（警告拒否）、Command・OC・文面・GitHub復元・受信基盤のSmokeを通過。登録なしのNative監視の起動/取消も確認した。廃止メンション試験の待機取消とunknown照合は既存OC Smokeへ含めた。LINEへの通知送信と本番配備は未実施。

2026-10-05、!pushsetting skdを同じconfigure / OC権限・登録上限へ追加し、statusはAndroid / iOS / skdを返す。run_store_monitorsはOSの5秒確認に加え、SKDの60秒確認を一つ起動する。SKDは専用HTTP・LINE Queueを作らず、更新本文と通知先を同じSQLiteへ保存して既存Outboxへ有限展開する。[履歴追跡・状態・スレッド配送](../../skd/docs/SKD.md)。現在の配備はProtocol v17のNativeとAdapterを同時に行う。

2026-10-06、record_storeはSQLiteの個別/全体BOT停止を照合し、停止したトークへ新しい更新通知を登録しない。停止中も検知版は同じtransactionで保存し、再開後に停止期間の更新をまとめて通知しない。停止前に登録済みのOutboxは変更しない。pushsettingのBOT権限はbot_rolesの最新値を参照する。[BOT管理の保存と権限](../../oc/docs/BOT.md)。

2026-10-08、run_store_monitorsへpush_loopを加え、Android・iOS・SKDと同じ取消・寿命でイベント／ガチャ通知を監視する。HTTP・Outbox・SQLite snapshotを共用し、store_subscriptionsへpushの個別IDを混在させず、自然な別設定tableを使う。!pushsettingとその権限は維持する。[pushの仕様と関数](../../push/docs/PUSH.md)。
