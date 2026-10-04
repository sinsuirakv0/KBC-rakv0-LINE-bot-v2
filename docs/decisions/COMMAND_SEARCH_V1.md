# Discord検索仕様とLINEの番号対話・Media Worker

2026-10-03。状態: 採用・実装済み。実素材の生成確認済み、実LINE確認は未完了。

ut/tut/stの仕様・資料・形態画像・file・motionをDiscord v2から引き継ぐ。LINEでは8候補への番号リプライと、👍で次/❤️で前という一覧へのリアクションを使う。本人・トーク・最新promptへ限定する。通常1〜3件のURLは1返信へまとめ、編集の代替を共通Sessionへ集約する。

2026-10-04変更: 利用者指定で次9/前0を廃止した。OCでは任意の絵文字リアクションを使えないため、標準のNICE / LOVEを採用する。通知にreactor MIDがないので必要な一覧だけ最大400件を追加照会する負担を受け入れる。旧ページを送信成功まで保持し、番号を未送信ページへ適用しない。[SDK確認・検証範囲](../../apps/line/docs/REACTIONS_AND_STICKERS.md)。

同梱snapshotから名称・IDを検索する。Discordと同じ正式データ・別称・分類range・共有形態を検証し、素材も同じassets commitへ固定する。検索時の外部通信と常時更新を避ける代わりに、名称・素材を更新するにはsnapshot再生成と再配備が必要。Discordの10分ごとの名前資料再検証は今回導入しない。snapshotが変わったSessionは失効する。

CPU描画・外部素材取得は通常配送から分け、既存SQLite OutboxへPrepareMediaとして永続化する。生成1件・未解決Media8件・期限10分、共有HTTP2枠・有限cache。受付/checkpointは生成結果を待たず同時commitする。内部ジョブを配送側へ渡さず、完成時に同じAction IDを通常Outboxへ変換する。この構成で生成中の同じトークでも軽いCommandの返信が進む。進捗ごとのLINE投稿を省き、受付・完成に絞る。

候補の擬似編集は新しい返信成功後の管理者削除。10分後の清掃と操作時の前倒しを同じ削除Actionへまとめる。LINEJS 3.4.2のOCメディア送信はOBS reqseq uploadが直接投稿する経路を使う。SDKのHTTP status不足を共通transportで検査し、通信開始後の結果不明を自動再送しない。

[Command仕様・関数](../../crates/kbc-core/src/commands/docs/COMMANDS.md)、[Worker](../../crates/kbc-core/docs/MEDIA.md)、[実素材実験](../../experiments/commands/docs/MEDIA_VERIFICATION.md)。
