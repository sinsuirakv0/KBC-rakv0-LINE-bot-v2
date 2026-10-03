# DiscordのMotion RendererをLINE Coreへ移植

2026-10-03。Discord v2 commit 02e6e9bのmotionモジュールを参照。描画アルゴリズムを引き継ぎ、Discord TaskRuntimeへの依存を共通Media Workerの小さなContext/Artifactへ置き換えた。入力・素材解決はcommands/search、取得はAssetService、実LINE送信はAdapterが所有する。

| モジュール・主要関数 | 働き・関係 |
| --- | --- |
| request::parse_motion_arguments | png/mp4/gif・形態・a/w/i/k・Frame/range・--fullを型付き有限Requestへ変換 |
| MotionPlan | 解決済みsprite/imgcut/mamodel/maanim path、segment、出力形式・scale |
| assets::load_motion_assets | 共有HTTP枠でsprite/cut/modelと最大4種のanimationを取得 |
| project::MotionProject::parse | cut・model・animationを検証。親参照・cycle・数値・個数を制限 |
| evaluate::evaluate | Frameの補間・親子変換・可視性からDrawPacketを作る。Discordの精度処理を再利用 |
| layout::resolve_frames/measure | segmentをFrame列へ展開し、2-passで表示範囲と有限layoutを決める |
| raster::SpriteSheet::decode/Rasterizer::render | Sprite/cutを一度展開し、再利用Pixmapへ合成する |
| MotionJob::render | Asset取得→blocking解析/計測→PNGまたはFFmpeg pipe。結果と測定をMedia Workerへ返す |
| render_mp4/render_gif | RGBAを逐次出力。前FrameとDrawPacketが同じなら描画を再利用。GIF paletteも有限sample |
| start_encoder/finish_encoder | 1threadのFFmpeg・stderr末尾8KiB・終了statusを管理。取消/失敗でkill、Windowsは非表示 |

30fps・最大900Frame、最大辺960px。PNGの面積640×480、動画480×400を上限とする。Spriteの展開は8M pixel、複製cut合計16M pixelまで。入力は資料1件4MiB、imgcut/modelの最大4,096項目、animation track16,384・key65,536を維持。上限を超える場合は無制限に拡張せず失敗案内にする。生成1件・8MiB、待機/実行期限・停止は [Media Worker](../../../docs/MEDIA.md) が所有する。

元Rendererの単体Testを大量複製せず、既存アルゴリズムを維持して実素材によるPNG・MP4・GIF・Frame数・durationとイベントループ応答を1つの [任意結合実験](../../../../../experiments/commands/docs/MEDIA_VERIFICATION.md) で確認した。NorthflankのCPU quota下の描画速度・RSSは未計測。

2026-10-03のメモリ対策: FFmpegは入力decoder・filter・出力encoderを1threadにし、frameは逐次pipeへ渡す。SpriteSheet.decodeは共通Contextへ展開予算を照会し、render_frameは16Frameごとにコンテナ使用量を確認する。MotionJob.renderは描画状態を明示dropしてから成果を確認し、container_peak_sample_bytesをstdout以外の診断ログへ出す。[Linux 0.2CPU / 512MiBでの実測](../../../../../experiments/motion-memory/docs/MEMORY.md)。
