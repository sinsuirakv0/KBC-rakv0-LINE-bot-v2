# Motionのメモリ観測

2026-10-03。利用者の運用報告を調査。Node RSSだけでFFmpegの子process分を把握したとは扱わない。

本環境の生成ログと機密のトークログから `!ut 710 motion mp4 f w i a k` を確認。345Frame・488×420、生成9.03秒、出力約1.4MB。累積入力RGBA 262,641,600byteはpipeへ逐次書いた合計であり、常駐メモリの測定値ではない。

同コマンドのWindows / Node24.15 / FFmpeg9.0.2 / Protocol v6実行では、parent peak55.5MiB、子FFmpeg peak31MiB、合計peak86.5MiB。Windowsのprocess working setを約100msごとに外部から取得した。LinuxのOOM回避を証明する値ではない。

任意のGitHub Actions `Motion memory probe` は本番Dockerfileをbuildし、0.2CPU / 512MiB / swap追加なしで同じ素材のMP4・GIF・PNGを生成する。LINE認証・通信は使わず、CoreのMedia Workerと公開ゲーム素材だけを使う。Bot常駐分の近似として110MiBのBufferを保持する。実Botとの完全な同一負荷ではない。

run.mjsは共通WorkerへCommandを投入し、成果・生成時間・Node maxRSS・cgroup memory.peakを記録する。measure.pyは各containerの終了codeとOOMKilledを保存し、失敗時にも結果artifactを残す。cgroup peakはFFmpegとfile cacheを含む。実測後に原因・対策と比較結果を追記する。

変更前のLinux実験 [37125913430](https://github.com/sinsuirakv0/KBC-rakv0-LINE-bot-v2/actions/runs/37125913430) は全形式成功、OOMKilled=false。MP4 4.88秒 / cgroup peak154.0MiB、GIF 7.00秒 / 209.7MiB、PNG 0.34秒 / 142.1MiB。110MiBの常駐近似を含む。素材710の再現条件ではメモリ異常を再現できず、本番症状が解消したとは判断しない。

対策はFFmpeg入力decodeも1threadに限定し、Rendererを成果照合より前にdropする。Media共通Contextのcheck_memoryはLinux cgroup使用量（子FFmpeg・file cache込み）を読み、コンテナ上限から64MiBを残した予算を確認する。Sprite展開前に展開・cut合計byteも加えて確認し、動画は16Frameごとに観測する。超過では生成だけを失敗案内にする。急激な増加を原子的に防ぐ硬い予約ではなく、入力の絶対上限・単一Workerと組み合わせる。非Linuxや制限値を取得できない環境では既存の入力上限を維持する。

追加の420MiB常駐近似では、OOMを起こさずMediaMemoryBudgetExceededで生成を中止できることを確認する。これは正常生成の成功とは別の検証で、expectedExitCode=1を記録する。
