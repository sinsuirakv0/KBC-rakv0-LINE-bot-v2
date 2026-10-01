# KBC LINE Bot V2

公開リポジトリ: [KBC-rakv0-LINE-bot-v2](https://github.com/sinsuirakv0/KBC-rakv0-LINE-bot-v2)

Rust CoreとTypeScriptのLINEJS Adapterを使うLINE Botの計画。

現在は計画段階。構成と設計原則はDiscord Bot v2、コマンドの仕様と運用知見は旧LINE Botを参照する。

LINEJSは最新公開版を採用する。2026-10-01の確認では `3.4.2` 。実装開始・依存更新時に公開版を再確認し、検証に使うversionをlockする。受信・通信の改善と残る検証は[SDK調査](docs/research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md#8-最新公開版の改善と採用方針)に記録する。

## 第一段階の目標

0.2コア・512MBで、多数のOpenChatのメッセージを取りこぼさず受信・処理し、API制限を避ける負荷制御と安定した応答・通知配送を実現する。応答時間と常時CPU使用率も計測して改善する。

第一段階はOpenChat専用。参加OCの各トークは原則すべて利用可能とし、個人・グループは後の段階でOC内から許可設定できる形にする。

コマンドprefixは `o.` 。入力例は `o.ping` 、 `o.help` 。旧版の `!` から変更する。

## 資料

- [移植計画](docs/plans/LINE_CORE_V2.md): 第一段階の設計・検証、機能移植の順序、完了条件。
- [構成と責務の案](docs/architecture/CORE_AND_ADAPTER.md): Rust / TypeScript境界、受信受付、API制御、状態の所有者。
- [旧版の調査・運用知見](docs/research/LEGACY_FINDINGS.md): 過去の対策、今回の障害報告、確認した実装と未確定の原因。
- [文書の設計原則](docs/engineering/DOCUMENTATION.md): 仕様・実装・設計判断・関数の関係を記録し、コードと一緒に更新する運用。
- [受信・常時処理の調査と実験](docs/research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md): LINEJSの実装確認、実施したオフライン検証、次に必要な比較。
- [旧コンテナでの受信実験](experiments/linejs-receiver/docs/LIVE_CONTAINER_PROBE.md): 既存アカウントで最新SDKの5分受信を実測。4トーク・新着2件を含む6メッセージ、API制限の観測なし。停止・切戻し済み。同時入力の照合は未完了。

[直近の着手順](docs/plans/LINE_CORE_V2.md#10-直近の着手順)のAは、最新SDK配布物での認証なし検証と、既存コンテナでの短時間受信実測を実施。次は[オフライン結果](experiments/linejs-receiver/docs/RECEIVER_PROBE.md)と実測を基に受付・復旧契約を具体化し、同時入力の照合を追加する。SDK既定PUSHの本採用は保留。Rust Core・返信・通知は未実装。
