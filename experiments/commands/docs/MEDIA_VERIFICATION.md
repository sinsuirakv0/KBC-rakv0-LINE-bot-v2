# Discordコマンド移植の素材生成実験

2026-10-03。Windows、Node.js 24.15.0、Rust 1.98 gnullvm release、FFmpeg/FFprobe 9.0.2。検索資料は [schema 2 snapshot](../../../data/search/docs/SNAPSHOT.md)。公開GitHub素材に通信し、LINEへは接続しない。対象は実装済みの検索→永続Media→Bufferまで。FFmpegのWindows配布元は [公式ダウンロード案内](https://ffmpeg.org/download.html) に掲載された [Gyan release](https://www.gyan.dev/ffmpeg/builds/)。zipの公開SHA-256を照合して使った。

| 入力 | 結果 | 生成実測 |
| --- | --- | --- |
| o.ut 0 motion png f a 0 | 416×520 PNG、1Frame、136,677byte | 素材1,711ms、解析/計測3ms、encode12ms、計1,727ms |
| o.ut 0 motion mp4 f w 0~~5 a 0~~5 | H.264 110×130、12Frame、400ms、8,243byte | 素材408ms、encode567ms、計976ms |
| o.ut 0 motion gif f a 0~~3 | GIF 104×130、4Frame、8,382byte | 素材249ms、encode165ms、計415ms |
| o.tut 0 motion png a 0 | 472×520 PNG、1Frame、119,256byte | 素材800ms、encode9ms、計813ms |
| o.ut 0 file f → 番号1 | 存在確認済み一覧・アイコン128×128 PNG、7,387byte | 取得・番号入力を通過 |

FFprobeで形式・寸法・Frame数を確認し、MP4 durationがCoreからAdapterへ渡す400msと一致した。PNGのネコ・わんこを目視確認。生成中に同じトークへo.pingを受付し、Core返信まで18.6ms。5ms timerは全実験中333回動いた。受信イベントループと通常配送が生成を待たないことをローカルで確認した。実LINE配送・PUSH受信そのものの性能試験ではなく、0.2core/512MiBで同じ数値を保証しない。

同日に最終版で再実行し、同じ形式・寸法・Frame数・成果byteを確認した。素材取得を含む合計はUT PNG 1,198ms、MP4 370ms、GIF 308ms、TUT PNG 697ms。生成中のpingは11.9ms、5ms timerは242回。これらは単発の実測で、取得時間やCPU性能の保証値ではない。オフラインSmokeでは8件中1件を古いsnapshot revisionへ変更し、未通信の再実行案内へ移ることも確認した。

再実行: npm run build後に絶対pathのFFMPEG_PATHを設定し、`node experiments/commands/verify-media.mjs`。出力はOSの一時ディレクトリ。一般Smokeへネットワーク依存を混ぜず、`npm run smoke` と `npm run smoke:commands` は認証もHTTPも使わない。後者はtxt/Discord資料、本人・最新リプライ・再開・期限、永続素材ジョブ復旧/失敗、OC upload境界/HTTP失敗/管理者削除の4経路をまとめる。

残る確認はコンテナbuildと復元、CPU quota下の生成、実OCでの画像・動画・GIF・rawファイルの表示、OBS objIdの扱い、管理者削除・API制限。2026-10-03にDocker CLIから確認したが、Docker Engineのpipeが存在せずbuildは実行できなかった。旧Bot・認証・GitHubの非公開保存データは変更していない。
