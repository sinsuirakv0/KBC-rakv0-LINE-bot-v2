# KBC LINE Bot V2

公開リポジトリ: [KBC-rakv0-LINE-bot-v2](https://github.com/sinsuirakv0/KBC-rakv0-LINE-bot-v2)

Rust CoreとTypeScriptのLINEJS Adapterを使うLINE Bot。

PUSH基盤・検索・OC管理を実装し、本環境の受信・保存復元を確認済み。今回追加したID取得・独立した参加退出取得の配備結果は [運用記録](docs/operations/MINIMAL_BOT.md) に残す。全件受信・長期安定性は引き続き観測する。構成・設計原則とut / tut / stはDiscord Bot v2、その他のコマンドと運用知見は旧LINE Botを参照する。

LINEJSは最新公開版を採用する。2026-10-03のnpm再確認でも `3.4.2`、採用配布物をlock。受信・通信の改善と残る検証は[SDK調査](docs/research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md#8-最新公開版の改善と採用方針)に記録する。

## 第一段階の目標

0.2コア・512MBで、多数のOpenChatのメッセージを取りこぼさず受信・処理し、API制限を避ける負荷制御と安定した応答・通知配送を実現する。応答時間と常時CPU使用率も計測して改善する。

第一段階はOpenChat専用。参加OCの各トークは原則すべて利用可能とし、個人・グループは後の段階でOC内から許可設定できる形にする。

コマンドprefixは `!` 。`!ping`・`!help`はtxtから読み込み、`!ut`・`!tut`・`!st`はID・名前検索、候補への番号リプライ、ut/tutのorigin・関連file・PNG/MP4/GIFのmotionに対応する。`!oc setup / join / leave / mute / kick / url / media / watch`も移植し、番号リプライで管理する。確認用 `!test-notify 5` も使える。`!` を既定、`o.` も別prefixとして受け付ける。

## 最小Bot

`!id` は自分・相手のMID、名前検索、トーク・親OC、観測済みのサブトーク投稿のリプライ情報を取得する。[利用範囲](crates/kbc-core/src/oc/docs/ID.md)。通常応答は普通の投稿、必要な副官通知ではリプライを使う。

OC PUSH → 取得ページの保存・ID重複排除 → RustのCommand → 共通API経路で返信する。通知確認コマンドはCoreの期限で起き、次の入力がなくても配送する。短時間RPCは全体2並列、同じ宛先の配送と同じcursorの取得は直列。

```sh
npm ci --ignore-scripts
npm run build
npm run smoke
npm run smoke:commands
npm run smoke:oc
```

認証の引継ぎ・起動・保存の制約は[最小Botの運用手順](docs/operations/MINIMAL_BOT.md)。現Northflankは永続Volumeなしのため、コンテナ交換時のCoreデータ保管と旧認証の受渡しを実験前に確定する。

ログは既存の非公開GitHubストレージを使う。2026-10-03の指定により旧ログも軽量形式へ変換し、`logs/v2/<sから始まるOC MID>/<mから始まるトークMID>/`へ整理する。[ログ保存方針](docs/decisions/OC_LOG_STORAGE_V2.md)に確定事項と未実装部分を記録した。旧ログの実削除時期は確認中。

## 資料

- [移植計画](docs/plans/LINE_CORE_V2.md): 第一段階の設計・検証、機能移植の順序、完了条件。
- [最小Rust Runtime](crates/kbc-core/docs/RUNTIME.md): SQLite、Command、期限起床、結果不明、関数と上限。
- [最小LINE Adapter](apps/line/docs/ADAPTER.md): PUSH、継続ページ、補完、API共通枠、認証と停止。
- [移植したCommand](crates/kbc-core/src/commands/docs/COMMANDS.md): ut/tut/st、番号リプライ、画像、関数・上限と検証。
- [OC管理コマンド](crates/kbc-core/src/oc/docs/OC.md): 旧権限区分、設定・参加通知・削除・処分、PUSHの条件、関数・上限。
- [txtによる応答とhelp](content/docs/CONTENT.md): ファイル追加で登録する方法と表示の規約。
- [検索データの更新](data/search/docs/SNAPSHOT.md): 同梱snapshotの出所・再取り込み・反映方法。
- [検索移植の判断](docs/decisions/COMMAND_SEARCH_V1.md): プレーンテキスト、有限Session、非同期受付、データ更新のトレードオフ。
- [新しいOCログの保存方針](docs/decisions/OC_LOG_STORAGE_V2.md): 旧ログの引き継ぎ中止、OC / トークの階層、ファイル集約と検索、同期・切り替えの実装順。
- [構成と責務の案](docs/architecture/CORE_AND_ADAPTER.md): Rust / TypeScript境界、受信受付、API制御、状態の所有者。
- [PUSHと有限並列の決定](docs/decisions/PUSH_AND_BOUNDED_CONCURRENCY_V1.md): PUSHを基本に、同じcursorの取得は直列、独立したトークの取得・処理・配送は有限並列にする方針。
- [旧版の調査・運用知見](docs/research/LEGACY_FINDINGS.md): 過去の対策、今回の障害報告、確認した実装と未確定の原因。
- [文書の設計原則](docs/engineering/DOCUMENTATION.md): 仕様・実装・設計判断・関数の関係を記録し、コードと一緒に更新する運用。
- [コマンド移植前のレビュー依頼](docs/engineering/FOUNDATION_REVIEW.md): 現在地、基盤コードの確認箇所、未実装、別のGPTへ渡す依頼文。
- [GPTレビューと照合結果](docs/research/FOUNDATION_REVIEW_RESULT.md): Chromeで完了したレビュー、先に修正する4点、容量再現、未確認事項と移植順序。
- [基盤の復旧契約と修正](docs/decisions/FOUNDATION_RECOVERY_V1.md): 送信開始境界、保存障害の停止、トーク別再試行、compact IDと容量の検証。
- [受信・常時処理の調査と実験](docs/research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md): LINEJSの実装確認、実施したオフライン検証、次に必要な比較。
- [PUSH受信で得られる情報](docs/research/PUSH_RECEPTION.md): 現在の定期取得との違い、参加・退出等のイベント一覧、全体通知とトーク別詳細、追加取得とAPI削減の条件。
- [旧コンテナでの受信実験](experiments/linejs-receiver/docs/LIVE_CONTAINER_PROBE.md): 既存認証・直列取得・資源使用量を実測。連続入力では完全一致 `!ping` 38件を取得し、全件照合・実返信は未評価。停止・切戻し済み。同時入力の手動試験は運用観測へ回す。

[直近の着手順](docs/plans/LINE_CORE_V2.md#10-直近の着手順)のAは、最新SDK配布物での認証なし検証と既存コンテナでの短時間受信実測を実施。B/Cの最小経路として3 crate・型生成・Native・PUSH Adapter・ping・期限通知を実装し、同時ID・重複・保存rollback・再開・自律期限・SDK sign-onと継続PUSHをオフライン検証。ChromeのGPTレビューの主要4点も修正し、送信境界・保存障害・トーク別再試行・保持容量を検証済み。次は認証・Coreの永続復元とDの少数OC実験。同時入力の手動試験は後続の運用観測へ回す。多OC・実API制限・実LINE返信の達成は未評価。

受信実験用には、試験OCへの有限な `pong` 返信と通知・トーク取得の比較経路を追加した。模擬検証と実コンテナの起動は確認済み。実LINEの試験入力との照合・返信確認は未完了。本BotのRust Command Runtimeとは区別する。

動画生成はFFMPEG_PATH（Linuxコンテナは/usr/bin/ffmpeg）を使う。名称・素材は同じ公開commitへ固定した同梱snapshotで、npm run snapshot:searchから更新する。[共通Media Worker](crates/kbc-core/docs/MEDIA.md) と [公開素材の生成確認](experiments/commands/docs/MEDIA_VERIFICATION.md) に関数・上限・再実行手順を記録した。

既存の非公開GitHubデータrepoから旧権限・OC設定を取り込み、永続Volumeなしで認証・Coreを退避・復元する。[保存・復旧の限界](docs/operations/GITHUB_RECOVERY.md)。旧ログは軽量形式へ変換する方針へ更新。[ログ設計](docs/decisions/OC_LOG_STORAGE_V2.md)。

本環境でPUSH受信と既存GitHubへの状態退避を確認した。ログはOC / トークの階層で発言・名前・参加退出を分け、同じ追記先を4MiBまで更新する。[新しい保存形式](docs/decisions/OC_LOG_STORAGE_V2.md)、[変換Workflow](scripts/logs/docs/MIGRATION.md)。
