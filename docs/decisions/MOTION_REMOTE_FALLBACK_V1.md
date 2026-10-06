# ut / tutのDiscord代行生成

2026-10-07（JST）。状態: 利用者指定の要件は採用。接続方式と切替対象の詳細は確認中。代行生成のコード・配備・通信検証は未実施。

## 確定した要件

`ut / tut motion`はまずLINE Bot側で生成する。メモリ不足などで生成できない場合、Discord Botへ同じ生成条件で依頼し、Discord Bot側のCPU・メモリを使う。LINE Botは生成済みのPNG / GIF / MP4を受け取り、既存のLINE配送経路で送信する。

LINE側へ未描画Frameや素材一式を返して生成を続ける形にはしない。依頼先のDiscord BotからLINEへ直接送信する形にもせず、LINEの送信結果・重複防止・成果削除は既存Outboxが所有する。

## 現行実装と変更箇所

| 関数・型 | 現行の働きと必要な変更 |
| --- | --- |
| `commands::search::motion_plan` / `MotionPlan` | 入力から素材path・形態・segment・形式・表示条件を解決する。同じ解決結果を代行依頼へ渡す |
| `Runtime::media_loop` / `RenderContext::check_memory` | 素材revisionを照合し、生成前にコンテナ使用量を確認する。現在はメモリ不足で失敗案内へ進むため、motionの代行切替経路が必要 |
| `Runtime::generate_media` / `MotionJob::render` | LINE上で素材取得・描画・FFmpegを実行する。生成失敗を分類し、対象となる失敗でDiscordへ依頼する |
| `Runtime::finish_media` / `prepare_attachment` | 完成ファイルを既存SendMessageへ変換し、配送時にBufferをAdapterへ返す。代行生成の成果もこの経路へ接続する |
| Discordの`MotionJob` / `TaskRuntime` | Rendererと有限実行枠は存在するが、外部のmotion依頼を受け付けて完成ファイルを返す経路は存在しない |
| Discordの`createEventUpdateServer` | 外部通知用HTTP窓口はあるが、motion生成には未対応。通知の認証・timeoutをそのままmotion用とみなさない |

参照したDiscordコードは`D:/KBC/KBC-rakv0-discord-bot-v2`の`crates/kbc-core/src/motion/`、`task_runtime.rs`、`commands/ut/mod.rs`、`commands/tut/mod.rs`、`apps/discord/src/external-events/server.ts`。この作業ではDiscord側の既存変更を編集していない。

## 実装時に守る境界

- メモリ不足の事前検知も代行切替へつなぐ。OOM killは同じプロセスでcatchできないため、強制終了後にその場でDiscordへ依頼できると扱わない。再起動時の未完了ジョブは既存の復旧契約と整合させる。
- ローカル生成の取消と実処理の終了を確認し、描画状態・FFmpegを解放してから代行する。終了しない生成を放置したまま代行や次のローカル生成を開始しない。
- 両Bot間は独立したversion付き契約とする。必要な生成条件と素材revisionを送り、LINEアカウントの認証情報やSDK Objectを渡さない。受信側は素材path・segment・形式・数値の有限性を検証する。
- Discord側の通常motionと代行生成は既存TaskRuntimeの容量・描画実行枠を共有する。代行専用の無制限QueueやRendererを作らない。満杯・生成失敗・依頼先停止を有限時間で失敗へ収束させる。
- 完成ファイルはLINEの既存作業領域へ保存する。受取中にも8MiB上限を検査し、形式・実ファイル・MP4のdurationを確認する。巨大なJSON / base64や全Frameの常駐展開を避ける。
- 完成後は同じLINE Actionを既存Outboxへ戻す。LINE送信開始後の失敗は既存の`unknown`規則を維持し、Discordへの再生成依頼で送信結果不明を解消しない。
- LINEとDiscordの描画処理には独立した更新があり、完全に同じbyteになるとは保証しない。代表入力で形式、指定Frame、形態、表示範囲、durationを照合する。

## 確認中の判断と提案

1. 接続方式: Discord側に認証付きHTTP APIを設け、LINE側の接続先URL・認証キーを環境変数へ置く案を提案。同一サーバーでファイル・プロセスを共有する方式かは利用者へ確認中。
2. 切替対象: メモリ不足に加え、FFmpeg失敗・生成timeoutなどの生成処理の失敗全般を対象とする案を提案。入力ミス・LINE送信失敗は対象外とする想定だが、利用者の回答前に確定しない。
3. HTTPを採る場合、同期応答とジョブ受付後の結果取得を、配備先の通信timeout・取消・重複依頼の扱いに合わせて決める。既存生成には待機・実行それぞれ10分の上限があるため、15秒・4MiBの素材取得設定をそのまま適用しない。素材revisionの固定方法と再起動時の代行状態も実装前に確認する。

HTTP案は別サーバーの資源を利用でき、完成ファイルだけを転送できる。一方、認証設定、依頼先の可用性、通信期限と成果の保持・削除を両Botで揃える必要がある。共有ファイル方式は転送を簡素にできるが、同一ホスト・共有領域への依存が生じる。

## 最小検証と完了条件

既存Smokeへ、ローカル成功時は代行しないこと、検知したメモリ不足から代行成果を同じLINE Actionへ渡せること、代行失敗・容量超過時に通常入力を止めないことを少数の条件として追加する。公開素材でPNG / GIF / MP4とdurationを照合し、実接続・実LINE送信はオフライン検証と区別して記録する。

現在は要件・変更箇所の調査まで。上記条件の成功や代行機能の実装完了を示す資料ではない。
