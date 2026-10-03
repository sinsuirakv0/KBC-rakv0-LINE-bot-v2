# Motionのメモリ観測

2026-10-03。利用者の運用報告を調査。Node RSSだけでFFmpegの子process分を把握したとは扱わない。

本環境の生成ログと機密のトークログから `!ut 710 motion mp4 f w i a k` を確認。345Frame・488×390、生成9.03秒、出力約1.4MB。累積入力RGBA 262,641,600byteはpipeへ逐次書いた合計であり、常駐メモリの測定値ではない。

同コマンドのWindows / Node24.15 / FFmpeg9.0.2 / Protocol v6実行では、parent peak55.5MiB、子FFmpeg peak31MiB、合計peak86.5MiB。Windowsのprocess working setを約100msごとに外部から取得した。LinuxのOOM回避を証明する値ではない。

任意のGitHub Actions `Motion memory probe` は本番Dockerfileをbuildし、0.2CPU / 512MiB / swap追加なしで同じ素材のMP4・GIF・PNGを生成する。LINE認証・通信は使わず、CoreのMedia Workerと公開ゲーム素材だけを使う。Bot常駐分の近似として110MiBのBufferを保持する。実Botとの完全な同一負荷ではない。

run.mjsは共通WorkerへCommandを投入し、成果・生成時間・Node maxRSS・cgroup memory.peakを記録する。measure.pyは各containerの終了codeとOOMKilledを保存し、失敗時にも結果artifactを残す。cgroup peakはFFmpegとfile cacheを含む。実測後に原因・対策と比較結果を追記する。
