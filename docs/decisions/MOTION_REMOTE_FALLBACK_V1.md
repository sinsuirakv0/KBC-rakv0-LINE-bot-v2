# ut / tutのDiscord代行生成

2026-10-07（JST）。状態: 認証付きHTTP APIと生成失敗全般からの切替を利用者が承認。両Botに実装し、ローカルHTTPで結合検証済み。本番設定・配備とDiscordのHTTPS生成を確認済み。実LINEでの代行成果の送信は未検証。

## 確定した要件

`ut / tut motion`はまずLINE Bot側で生成する。メモリ不足などで生成できない場合、Discord Botへ同じ生成条件で依頼し、Discord Bot側のCPU・メモリを使う。LINE Botは生成済みのPNG / GIF / MP4を受け取り、既存のLINE配送経路で送信する。

LINE側へ未描画Frameや素材一式を返して生成を続ける形にはしない。依頼先のDiscord BotからLINEへ直接送信する形にもせず、LINEの送信結果・重複防止・成果削除は既存Outboxが所有する。

## 現行実装と変更箇所

| 関数・型 | 現行の働きと必要な変更 |
| --- | --- |
| `commands::search::motion_plan` / `MotionPlan` | 入力から素材path・形態・segment・形式・表示条件を解決する。同じ解決結果を代行依頼へ渡す |
| `Runtime::media_loop` / `RenderContext::check_memory` | 素材revisionを照合。motionのメモリ確認はローカル試行内で行い、生成前の不足も代行へ切り替える |
| `Runtime::generate_media / render_motion` / `MotionJob::render` | ローカル試行の失敗を分類。終了確認後に代行する。再起動時は保存した試行状態から代行を再開する |
| `RemoteMotion::render / download` / `AssetService::motion_request` | 同じClient・HTTPの2枠で依頼・結果照会・成果受取・取消を行い、完成ファイルを既存作業領域へ保存する |
| `Runtime::finish_media` / `prepare_attachment` | 完成ファイルを既存SendMessageへ変換し、配送時にBufferをAdapterへ返す。代行生成の成果もこの経路へ接続する |
| Discordの`RemoteMotionService` / `MotionJob` / `TaskRuntime` | 外部依頼・成果を2件まで保持し、通常Taskと同じ受付2件・描画1件の枠で生成。成果だけを外部入力境界へ返す |
| Discordの`createEventUpdateServer / createMotionHandler` | 既存HTTP窓口へ独立認証のmotion APIを接続。HTTP入出力は最大4件、生成状態はRustが所有する |

参照したDiscordコードは`D:/KBC/KBC-rakv0-discord-bot-v2`の`crates/kbc-core/src/motion/`、`task_runtime.rs`、`commands/ut/mod.rs`、`commands/tut/mod.rs`、`apps/discord/src/external-events/server.ts`。この作業ではDiscord側の既存変更を編集していない。

## 実装時に守る境界

- メモリ不足の事前検知も代行切替へつなぐ。OOM killは同じプロセスでcatchできないため、強制終了後にその場でDiscordへ依頼できると扱わない。再起動時の未完了ジョブは既存の復旧契約と整合させる。
- ローカル生成の取消と実処理の終了を確認し、描画状態・FFmpegを解放してから代行する。終了しない生成を放置したまま代行や次のローカル生成を開始しない。
- 両Bot間は独立したversion付き契約とする。必要な生成条件と素材revisionを送り、LINEアカウントの認証情報やSDK Objectを渡さない。受信側は素材path・segment・形式・数値の有限性を検証する。
- Discord側の通常motionと代行生成は既存TaskRuntimeの容量・描画実行枠を共有する。代行専用の無制限QueueやRendererを作らない。満杯・生成失敗・依頼先停止を有限時間で失敗へ収束させる。
- 完成ファイルはLINEの既存作業領域へ保存する。受取中にも8MiB上限を検査し、形式・実ファイル・MP4のdurationを確認する。巨大なJSON / base64や全Frameの常駐展開を避ける。
- 完成後は同じLINE Actionを既存Outboxへ戻す。LINE送信開始後の失敗は既存の`unknown`規則を維持し、Discordへの再生成依頼で送信結果不明を解消しない。
- LINEとDiscordの描画処理には独立した更新があり、完全に同じbyteになるとは保証しない。代表入力で形式、指定Frame、形態、表示範囲、durationを照合する。

## 採用方式・起動・期限

LINE側に`MOTION_REMOTE_URL=https://<Discord側のHTTP公開先>/motion-jobs`と`MOTION_REMOTE_SECRET`を両方指定する。Discord側の`MOTION_RENDER_SECRET`を同じ32〜256文字の印字可能ASCIIにする。未設定では従来のローカル生成・失敗案内を維持する。片方だけの設定や不正なURL / keyは起動失敗にする。LINEは公開先のHTTPSを必須とし、localhost等のローカル検証だけHTTPを許可する。SecretをURL・ログ・公開Gitへ入れない。

2026-10-07、本番の接続先を`https://p01--kbc-discord-bot--xwtq22smkhqt.code.run/motion-jobs`として設定した。Discordの既存公開ポート3000・HTTPS domainを使い、共通キーはNorthflankの各サービスの環境変数へ保存した。[配備と実接続の確認範囲](../../experiments/motion-fallback/docs/VERIFICATION.md#本番設定とhttps検証2026-10-07)。

代行Protocol v1は`POST /motion-jobs`で受付、`GET /motion-jobs/:id`で状態、`GET /motion-jobs/:id/artifact`で成果、`DELETE /motion-jobs/:id`で取消・削除する。依頼IDは既存LINE Action IDのSHA-1で、本文はversion・依頼ID・素材参照revision・解決済みMotionPlan。元のコマンド本文・OC MID・LINE認証情報を送らない。主な契約はDiscord側`crates/kbc-core/src/motion/docs/REMOTE.md`に記録した。

切替対象は事前メモリ不足、素材取得失敗、描画失敗、FFmpeg未設定・失敗、ローカル実行timeout。入力解析・不正なFrameや素材データ、検索revision不一致、LINE送信失敗では代行しない。ローカル成功は代行APIを呼ばない。

生成前に`actions.code=MotionLocalRunning`、代行前に`MotionRemotePending`を保存する。再起動でpreparingがqueuedへ戻ったとき、どちらのcodeでもローカル重処理を繰り返さず代行する。依頼の同じID・同じ内容はDiscord側で再受付せず、異なる内容は409。通信切断で受付結果が不明なら同じIDの結果だけを照会する。Discord再起動で結果が失われた場合はLINEの次の再開で同じIDを再受付できる。LINEの送信成功・unknownの扱いは既存配送経路が所有する。

ローカル実行は10分、取消後の終了猶予15秒。代行の待機・実行は各10分、LINEの代行待ち全体は20分30秒、状態照会は2秒間隔・通信失敗3回で終了する。代行を有効にしたmotionのMedia Worker全体は31分を上限とする。未着手の待機期限は10分。保存済みの生成状態からの再開は待機期限で拒否せず、作成時刻から待機10分＋実行31分の残り時間までとする。他の素材・スケジュールの期限は従来どおり。終了しないローカル実処理はMedia Worker異常として停止し、代行を並行開始しない。

各HTTPは共通Clientの15秒・2枠を使い、生成完了の間はHTTP枠を保持しない。JSONは16KiB、成果は受取中も8MiBで制限し、形式のヘッダ・先頭byte・名前・MP4 durationを確認してディスクへ逐次保存する。LINE側でFrameや画像を再展開しない。成功後は代行成果を削除、取消・失敗時も最大3秒で取消を伝える。受付を明示的に拒否された場合は既存の別依頼を削除しない。

受付後に結果を取得するHTTP方式は別サーバーの資源を利用でき、長い生成中もHTTP接続を保持しない。一方、接続先と認証設定、依頼先の可用性、成果の保持・削除を両Botで揃える必要がある。main参照は生成時点の公開内容へ依存し、固定commit以外のbyte一致は保証しない。

## 最小検証と完了条件

任意の[結合実験](../../experiments/motion-fallback/run.mjs)で両Native・ローカルHTTP・公開素材を使い、ローカル成功、生成途中の再起動、FFmpeg不足からの代行、PNG / GIF / MP4の復号・Frame数・duration、生成中のping、認証・入力・重複・容量上限、成果上限と失敗後のpingを確認する。LINE / Discordへの実投稿を行わない。Linuxのcgroup不足、実際の10分timeout、別サーバーへの公開・実LINE送信・本番負荷は別の検証として残す。

LINEJSの最新公開版は実装開始時のnpm registry照会でも3.4.2。依存・lockは変更していない。検証結果は[実験記録](../../experiments/motion-fallback/docs/VERIFICATION.md)へ残す。
