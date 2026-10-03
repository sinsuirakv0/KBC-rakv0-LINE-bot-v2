# 有限な素材・動画の永続Worker

2026-10-03。Rust Core共通のMedia Workerを実装。検索origin/file/motionが同じ仕組みを使う。

受付は `MediaRequest` を `PrepareMedia` ActionとしてSQLiteへ保存し、Event・Session・checkpointと同じtransactionでcommitする。以後は元のCommand本文を必要としない。通常のnext_actionはPrepareMediaを除外し、run_media_jobsが1件ずつ `queued → preparing` として取得する。preparingは通常のトーク直列配送を占有しないため、同じトークのpingも配送できる。処理後は同じAction IDでSendMessageへ変換して既存Outboxを起こす。

| 関数 | 責務と関係 |
| --- | --- |
| run_media_jobs/media_loop | Workerは同時1呼出。空なら共通Notifyを待ち、素材準備・描画を順に実行 |
| generate_media | Download、FileList、Motionを共通AssetService / MotionJobへ接続 |
| finish_media | 成果を既存配送Actionへ変換。file確認ではSessionの現在actionも照合し、古い結果で上書きしない |
| prepare_attachment | claimedの成果を行番号から読む。ファイルpathをAdapterやCommand入力に渡さずBufferだけを返す |
| AssetService | 固定commit・相対path・HTTPSを検証。Client/2 HTTP枠、15秒、4MiB、HEADの正負cache512件/10分を共用 |
| RenderContext | 作業ディレクトリ・取消・待機時間だけをRendererへ渡す。進捗ごとのLINE投稿は行わない |
| prune_media/complete_action | 再起動時に参照のない作業領域を削除。sent/確定failedの成果を削除し、unknownは照合まで保持 |

未解決Mediaは準備待ち・生成中・配送待ち・unknownを合わせ8件。Outbox全体2,048件の内数。満杯ならそのCommandを混雑案内1件に変え、通常入力の受付を維持する。待機10分・実行10分、取消後15秒の終了猶予。終了しない実処理を新Workerで置き換えず、異常として停止する。停止時は取消を伝え、未通信のpreparingをqueuedへ戻す。再起動でもpreparing/claimedは再待機、sendingはunknownへ分ける。

作業領域はCore DBの隣のmedia/job-行番号。成果は最大8MiB、最大8未解決成果で64MiB。描画時に素材・palette・出力の一時領域も必要になる。成果をDBのJSON/base64へ入れない。準備失敗・待機期限・生成上限は未通信なので通常の失敗案内へ変換する。取得済み添付の通信開始後は既存のunknown規則を使う。

LINEJS 3.4.2のOCメディア送信は `obs.uploadObjTalk(chat,type,blob,undefined,filename,duration)`。oid省略のreqseq方式でOBS自身が投稿する。空のIMAGE/VIDEOを先に予約しない。共通ApiSchedulerのuploadMedia枠・実fetch直前でsendingを記録し、SDKが検査しないHTTP statusを補う。テキストはsendMessage、管理者削除はdestroyMessage。メディア自体はrelatedMessageId付きリプライにならず、先行する案内を元Commandへの返信として出す。動画のdurationは描画したFrame数/30fpsから渡す。

FFmpegの実行pathはCoreConfig.ffmpegPath（FFMPEG_PATH）。Linuxコンテナは/usr/bin/ffmpeg、Windowsは絶対pathを指定する。DockerにFFmpegを追加したがDocker build・0.2core/512MiB環境・実LINEアップロードは未確認。ローカルで [公開素材のPNG/MP4/GIF・file実験](../../../experiments/commands/docs/MEDIA_VERIFICATION.md) と、通信なしのジョブ復旧・OBS例外・削除失敗を確認した。

未実行ジョブにもsnapshotのrevisionを保存し、更新後の索引・共有素材へ以前の解決結果を適用しない。revision不一致は未通信の再実行案内に変える。配送直前に成果が消失していても、通常返信へ変えて他の配送を維持する。fileの選択案内には固定commitのダウンロードURLも添える。

RenderContext.check_memoryはLinuxコンテナの使用量を共通Workerで確認し、上限の64MiB手前で新規生成・Sprite展開を拒否する。動画では16Frameごとに再確認し、子FFmpegも含む使用量のsample最大値を記録する。取消・入力上限は従来どおり維持する。[条件・限界とLinux実験](../../../experiments/motion-memory/docs/MEMORY.md)。
