# Discord代行生成の結合検証

2026-10-07（JST）。Windows、Node.js 24.15.0、Rust 1.98.1。LINE Core Protocol v18、Discord Core Protocol v7、独立したMotion代行Protocol v1を使用。対象実装はLINE Core 0de44aa・起動設定ccd3fa8、Discord Core bcf1ad5・HTTP / Native e25e5be。Discord側の作業前から存在した未commitのlayout変更を含む作業ツリーで検証し、その変更には手を加えていない。採用版LINEJS 3.4.2はnpm registryで再確認し、依存・lockは変更していない。

## 条件と再実行

仮説: LINEが生成できなくても同じMotionPlanをDiscordへ渡し、完成ファイルだけを既存配送Actionへ戻せる。実LINE / Discordの認証・投稿は使わず、両Native、127.0.0.1のHTTP、公開ゲーム素材、FFmpegで確認する。Secretは毎回ランダム生成し、記録しない。

1. 両repoで`npm run build`を実行する。
2. LINE repoから`node experiments/motion-fallback/run.mjs`を実行する。別配置では`DISCORD_BOT_ROOT`を指定する。FFmpegはDiscordのffmpeg-staticを使用し、任意の`FFMPEG_PATH`でも変更できる。
3. 最終JSONの`ok=true`と、出力ファイルを保存した一時directoryを確認する。常設テストや実Bot起動には組み込まない。

`submit / take / sent`がfixtureの受信・Action取得・配送完了を接続し、`artifact`がLINEの`prepareAttachment`で得た成果を保存してFFmpegで復号する。`post`とローカルサーバーが認証・重複・容量・受取上限を確認する。終了時に両CoreとHTTPを停止する。

## 実測結果

ローカルHTTPの結合実験は成功。PNGは1Frame、MP4は6Frame・200ms、GIFは3Frameとして復号した。

| 成果 | 条件 | byte | Frame |
| --- | --- | ---: | ---: |
| resumed.png | 生成中のpreparingとMotionLocalRunningを保存し、作成時刻から11分経過を再現して再起動。DiscordでPNG生成 | 136,677 | 1 |
| local.png | LINEでPNG生成成功。代行POSTを増やさない | 136,677 | 1 |
| delegated.mp4 | LINEのFFmpeg不足から切替。移動3Frame＋攻撃3Frame | 5,963 | 6 |
| delegated.gif | tut、LINEのFFmpeg不足から切替。攻撃3Frame | 9,069 | 3 |

- 生成中と成果上限拒否後のpingを配送Actionとして取得。
- 不正なFrame指定は失敗案内へ変換し、代行POSTを増やさない。
- 誤認証401、契約version・素材path不正400、同じID・同じ本文202、異なる本文409を確認。
- 外部の保持2件に達すると3件目は429。完成成果を取得・DELETE後は状態404。
- 8MiB超のContent-Lengthを持つ成果をLINE側で拒否し、通常入力を継続。

各数値は公開素材と今回の描画版に対する結果。LINEとDiscordの描画更新が別々のため、全入力のbyte一致を保証する値ではない。

## Compiler・既存の最小検証

- 両repoの`npm run build`成功。
- LINEの`cargo clippy --locked --workspace --all-targets -- -D warnings`成功。ビルド用LLVM MinGWのPATH、linker、LIBNODE_PATHは`scripts/native.cjs`と同じ設定を使用。
- Discordの`node scripts/run-cargo.js clippy --locked --workspace --all-targets`成功。`-D warnings`は既存のcommand_runtime、botstatus、commands、motion/rasterの警告で失敗。今回の変更箇所に新規警告はない。
- Discordの`node scripts/run-cargo.js test --locked -p kbc-core task_runtime -- --nocapture`は4件成功。取消終了待ち、進捗経路終了、添付成果後の掃除、panic掃除を確認。
- LINEの`npm run smoke:commands`、Discordの`node apps/discord/dist/protocol/smoke.js`成功。実Bot通信は使わない。

## 検証範囲

ローカル実験では保存状態と時計を操作した再起動復旧、FFmpeg不足、成果上限を再現した。Linux cgroupの実メモリ圧迫・OOM kill、実際の10分timeout、実LINE upload・送信、継続負荷は未検証。本番設定・配備とHTTPS公開先での確認は以下に分けて記録する。

## 本番設定とHTTPS検証（2026-10-07）

ChromeからNorthflankの同じproject内にある`kbc-discord-bot`と`kbc-rakv0-line-bot`の環境変数を設定した。既存のDiscord公開ポート3000とHTTPS domainを使い、LINEの`MOTION_REMOTE_URL`を`https://p01--kbc-discord-bot--xwtq22smkhqt.code.run/motion-jobs`とした。ランダムな64文字の共通キーをDiscordの`MOTION_RENDER_SECRET`・LINEの`MOTION_REMOTE_SECRET`へ保存。既存変数を保ち、値の一致を入力画面で確認してUpdate onlyで保存した。キーの値はGit・資料へ残さず、HTTPS検証用の一時ファイルは使用直後に削除した。

| Bot | 配備版 / build | Linux build | 最終状態 |
| --- | --- | --- | --- |
| Discord | e25e5be / uplifting-shade-921 | 2分10秒・Success | Running、1 / 1、0.2vCPU / 512MB |
| LINE | 6723105 / near-slope-9679 | 2分44秒・Success | Running、1 / 1、0.2vCPU / 512MB |

Discordは0 instanceへ変更後に新buildを配備し、1へ戻した。LINEは一時的にCDをOFFにし、旧版を動かしたままbuildした。build成功後に0 instanceへ変更、旧版の0 / 0を確認して新buildを配備し、CDをON・1 instanceへ戻した。開始時と終了時のCI / CDは両BotともON。資源・domainの追加購入はない。配備には今回のcommitだけをpushし、Discord側に元から存在した未commitの変更は含めていない。

HTTPSの外部APIを直接確認し、未認証401、正しい認証で202受付・ready・完成成果GET・DELETEを確認した。PNG / MP4 / GIFのContent-Type・名前・Protocol v1・先頭byte・8MiB上限を検査し、MP4のdurationも確認。今回のHTTP確認では動画の再復号は行っていない。

| 成果 | Frame指定 | byte | duration |
| --- | ---: | ---: | ---: |
| PNG | 攻撃Frame 0の1Frame | 136,677 | 対象外 |
| MP4 | 攻撃Frame 0〜2の3Frame | 4,421 | 100ms |
| GIF | 攻撃Frame 0〜2の3Frame | 9,125 | 対象外 |

LINEは配備後にhealth 200 / state=receivingを確認。停止前・再開後ともCoreのcompletedActions=100、unknownActions=25、retainedEvents=805。queued / claimed / querying / sending / preparingMedia / pendingLogsは0で、保存状態の復元を確認した。unknownを削除・成功へ変更していない。

Bot投稿を作らずにDiscordの実資源で生成した結果であり、実OCでLINEがメモリ不足になって代行成果を送信する流れ、長いmotionの負荷、OOM再現の確認とは区別する。今回の検証によるLINE / Discord投稿は0件。

[採用仕様・期限・接続設定](../../../docs/decisions/MOTION_REMOTE_FALLBACK_V1.md)
