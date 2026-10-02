# 最小LINE Adapter

作成日: 2026-10-02。状態: TypeScript build・模擬PUSHの検証済み。実LINEのPUSH購読、返信、通知、長期復旧は未確認。

## 関数と処理経路

`main → AuthStorage.load → SDK login → createCore → Receiver.run + 2本の配送loop`。

| 関数・状態 | 働き・相互関係 |
| --- | --- |
| `AuthStorage` | SDKの認証・reqseqを直列更新し、tmp書込・fsync・renameで保存。成功後だけメモリを更新。保存エラー・64件の待機上限到達をglobal abortへ伝え、以後の読書きを拒否。`flush`は全保存を待つ |
| `installApiScheduler` | 3.4.2の`requestCore`を包み、通常・LEGY・SDK token更新を共通枠へ接続。RPC本文の処理完了まで枠を持つ |
| `ApiScheduler.run / pace / pump` | 最大2実行・32待機のFIFOと開始間隔250ms。enqueue・完了・cooldown終了で自律的に起きる。初期値はLINEの許容制限値ではない |
| `Receiver.session` | OCのservice 3だけをHTTP/2で購読。初期応答をSDK Thriftで読み、SDK既定Stream・先行cursor更新を使わない |
| `accept` | SDK Eventをplain DTOへ変換。sender・REPLYの返信先もDTOへ正規化し、非同期Nativeのcommit後だけSDKの再接続用syncを進める |
| `completePending / drainChat` | 必要と示されたchatだけ最大2系統、1回4ページずつ補完。同じchatは直列。失敗をトーク別の期限付き再試行へ変え、位置を残す |
| `deliverAction` / 配送loop | `nextAction`で`claimed`を取り出す。API枠・reqseq保存・通信準備後、`client.fetch`直前の`beforeFetch`で`markSending`する。通信前失敗は再待機、通信後の例外は`unknown`。次の受信入力を必要としない |

PUSHはdirtyフラグに集約し、通知ごとにTaskやQueueを増やさない。account cursorを取得・保存するloopは一つ。取得中・継続ページ中に来たdirtyは全ページ終了後の再取得まで残す。補完待ちchatと`chatRetries`（試行数・再試行時刻）を全体checkpointへ保存し、補完終了後だけ消す。最大256トークを持ち、上限では黙って捨てず停止。最初のchat取得でも保存した初回originより前の履歴Commandは実行しない。

局所補完の失敗は2秒から最大15分のbackoffにし、account PUSHを開始・継続する。未完了の継続ページは待ち列の末尾へ戻し、新着へ譲る。PUSH待機中は最も早い補完期限でも起き、chatだけを再取得する。補完のために空の`fetchMyEvents`を追加しない。`pendingChats` / `chatFailures`で部分障害を観測し、必要ならローカルcheckpointから対象を照合する。

PUSH接続・ACKはRPC枠を占有しない。初期sign-onも間隔制御を通し、回数は`receiver.signOns`へ記録。短時間RPC回数は`api.methods`で観測する。PUSH通知、継続ページ、要求されたchat補完、subscriptionのttl期限（80%地点）で取得する。ttl不明時は30分を仮値とし、全OCの固定高頻度巡回は行わない。

HTTP 429の数値Retry-After、`EXCESSIVE_ACCESS`等の識別できる制限応答は全体cooldownへ反映。既定60秒、最大600秒。SDK token更新・再要求は親RPCの枠内で実行し、枠の循環待ちを避ける。RPC名のAsyncLocalStorageにより、送信に伴うrefresh通信を送信開始として扱わない。通信前の再待機以外にアプリ独自の送信再試行はしない。数値だけの未知codeや恒久的アカウント制限の正規化は今後の観測対象。

## 接続・容量・停止

HTTP/2の初期接続とsign-on待ちは各15秒。PUSH入力の組立Bufferは1MiB、SDKへ書込む未消費のBufferは64KiBを上限とし、ACK失敗の未監視rejectをcatchする。無フレーム90秒で接続を閉じる。定期チェックは30秒、keepaliveの`noop`も共通RPC枠へ通す。

各取得は100件、accountの連続100ページで一度復旧へ戻り、保存済みcontinuationから再開する。補完の指示は1ページ最大100chat、持ち越しと合わせ最大256chat。通常接続復旧は1〜60秒のbackoffと小さなjitter。旧接続と取得が終わるまで次の接続を開始しない。Core保存失敗・checkpoint破損・容量超過は局所復旧へ逃がさずプロセス停止する。

終了はglobal signalで通信を取消し、Coreの待機を起こす。`claimed`は次回起動で再待機、`sending`は`unknown`になり自動再投稿されない。Receiverはreadとkeepalive noopをjoinし、mainは受信・配送終了後にStorage全体の`flush`を待つ。flush失敗でもhealth server等の後始末は実施する。停止の強制上限20秒。SDK内部のLEGY通信はSDK自身の15秒timeoutで取消す。DBと外部通信は原子的でないため、送信開始記録直後の終了は結果不明になり得る。

## 今回の対応範囲と観測

コマンドはtxtの応答・help、ut/tut/stの検索・番号リプライ・origin画像、確認用test-notify。[Command実装](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)を参照。ping本文は旧LINEのpong!、登録と引数解析はDiscordのcatalog方式。個人・グループ、LINE thread固有メッセージ、権限・停止設定、旧通知機能は未対応。

未知・非テキストイベントは種別と件数を最大64種で観測する。ログには本文・トークID・message ID・認証値・SDKの生の例外を出さない。受信後の詳細はローカルSQLiteに残る。ログはstdoutの集計と配送結果のみ。毎分CPU（1core比）、RSS・heap、API・受信・配送・容量を出す。`/health`は受信ready時200、それ以外503。PUSH heartbeatだけで全メッセージの受信成功を保証しない。

LINEJSの最新公開版は2026-10-02も3.4.2。npm配布物revision 11をlock。そこでreqseq初回並列の直列化を確認した。SDKが依存するThrift 0.20はnpm auditでhighが出たため、0.25.0へoverride。実SDKのCompact Protocol初期応答を模擬検証し、audit 0件を確認。通信先の実互換性は少数OCの実験で確認する。

## Commandの追加境界

Protocol v3。通常返信の実送信message IDをCoreへ渡し、候補の受付先へ結び付ける。DeleteMessageは管理者権限のsquare.destroyMessageへ渡し、共通transportの直前にsendingを記録する。Coreが返信成功を確定してから削除を配送するため、削除失敗で新promptを巻き戻さない。画像はCoreで取得し、SDKのIMAGE送信とOBS uploadを共通API枠へ通す。SDK uploadがHTTP statusを検査しないため共通transportで補う。通常fetchは15秒の取消上限を持つ。LINE通信後のupload例外もunknownで、画像placeholderを自動再投稿しない。
