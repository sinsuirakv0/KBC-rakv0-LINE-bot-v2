# コマンド移植前の基盤レビュー

作成日: 2026-10-02。状態: Chromeの通常GPTによるレビューとローカル照合が完了。指摘の修正は未実装。[結果と修正候補](../research/FOUNDATION_REVIEW_RESULT.md)を参照。

## 依頼実行の記録

2026-10-02、利用者が開いたサイドバーのChatGPTで通常のChatモードを使い、GitHubプラグイン付きで公開コードの読み取り専用レビューを依頼した。Workへの切り替え、思考速度の変更、生成の停止は行っていない。

画面は`Unknown error`と「ChatGPT モデルを読み込めませんでした」を表示した。終了済みの生成を一度再試行し、間隔を空けて確認したが同じエラーで終了した。この作業チャットからのGitHubプラグインによる依頼資料の取得は成功した。画面には原因の詳細がなく、GPTがコードを読んだことやレビューを完了したことは確認できない。レビュー対象のコードは変更せず、以下の依頼資料を再利用できる状態にしている。

利用者の指定でChromeへ移り、通常ChatのGitHubプラグインから同じ基盤コードのレビューを依頼した。思考中は返答の有無だけを間隔を空けて確認し、設定変更・停止・追加の催促は行っていない。「15m 56s考えました」と回答完了を確認した。送信境界、認証保存障害、トーク補完の全体停止、保持上限の4点をソースへ照合し、保持上限は通信なしで再現した。未確認のSDK差分と既知の未実装は[結果資料](../research/FOUNDATION_REVIEW_RESULT.md)で区別する。

## 対象と現在地

リポジトリ: [KBC-rakv0-LINE-bot-v2](https://github.com/sinsuirakv0/KBC-rakv0-LINE-bot-v2)。基盤コードの対象commit: `6027cfca04bc21217ca7d6eb57fce7943b8b0fd9`。

Rust Core / Protocol / Native Bridgeと薄いTypeScript Adapterを実装済み。OC PUSH、継続ページと必要なchat補完、SQLiteでの受付・Action・checkpointの同時保存、ID重複排除、期限通知、全RPCの共通有限枠を接続している。コマンドは`o.ping`、`o.ping help`、確認用`o.test-notify`。

Native build、型・整形・Clippy、オフラインSmokeを確認した。Smokeの範囲は異なるID、重複、Batch rollback、再開、結果不明、自律期限、所有者、SDK token更新時の有限枠、実SDK Thrift sign-on、継続中のPUSH通知、chat補完。新Botの実LINE配備・長時間負荷・本運用での制限回避は未確認。

## 先に読む資料とコード

`AGENTS.md`を読んだ後、次の組合せで実装と説明を照合する。

| 調査対象 | 主なコード | 資料 |
| --- | --- | --- |
| 受付・期限・配送状態 | `crates/kbc-core/src/lib.rs` | [Runtime](../../crates/kbc-core/docs/RUNTIME.md) |
| 型とNative境界 | `crates/kbc-protocol/src/lib.rs`、`crates/kbc-node/src/lib.rs`、`apps/line/src/protocol/native.ts` | [全体構成](../architecture/CORE_AND_ADAPTER.md) |
| PUSHとページ再開 | `apps/line/src/adapter/receiver.ts` | [Adapter](../../apps/line/docs/ADAPTER.md) |
| API枠と制限待機 | `apps/line/src/adapter/api.ts` | [有限並列の決定](../decisions/PUSH_AND_BOUNDED_CONCURRENCY_V1.md) |
| 認証・起動・終了 | `apps/line/src/adapter/storage.ts`、`apps/line/src/main.ts`、`Dockerfile` | [運用制約](../operations/MINIMAL_BOT.md) |
| 検証の範囲 | `apps/line/src/smoke.ts` | [移植計画](../plans/LINE_CORE_V2.md) |

LINEJSとの接点はpackage-lockで固定した3.4.2配布物を照合する。SDKの内部境界を包んでいる箇所と、Thrift 0.25.0 overrideの実互換性も確認対象。過去の受信Probeと、今回のRust Coreを含む最小Botの実測を混同しない。

## レビューの重点

1. **取りこぼしと復旧**: PUSH到着と取得・待機の競合、初期sign-on、continuation、補完待ち、保存失敗・再接続・再起動で取得位置を飛ばさないか。SDKのACKを永続受付の成功として扱っていないか。
2. **自律配送と順序**: 次の入力なしでTimer・空いた配送枠が起きるか。同じトークの送信順、他トークの進行、送信中の終了、結果不明の扱いに矛盾がないか。
3. **API制限と取消**: 通常・LEGY・token更新・keepalive・PUSH再接続が適切な枠と開始間隔を通るか。cooldown中の開始予約、HTTP 429、Queue上限、取消時の枠回収を確認する。
4. **有限性と長期運用**: 0.2コア・512MBで、Event 8,192・Action 2,048・48時間保持・SQLite page上限がどの負荷で満杯になるか。未解決Action、Buffer、認証保存待ち、Native同期処理のCPU・Memory・停止への影響を確認する。
5. **停止と障害境界**: 認証保存失敗、接続初期化中の終了、読取・ACK失敗、health照会、20秒の終了上限で仕事や接続が残らないか。意図した停止と局所復旧の境界を確認する。
6. **移植の土台**: 現在のDTOは本文・chat ID・message ID・時刻のみ。送信者・OC MID・mention・権限に必要な拡張点を確認する。重いCommandを受信transaction内へ追加せず、小さい共通実行部へ移せるか。

## 既知の未実装と次の順序

GitHubからの旧認証自動復元・新Runtimeの退避、コンテナ交換時の永続性、一般Command Worker、権限・停止設定、長期ログ、threadは未実装。これらは「存在するはずの機能の不具合」と区別し、どの移植・実運用を始める前に必要かを判断する。

旧ログは移行せず新規開始し、OC MID（s）配下にトークMID（m）を置く。[保存方針](../decisions/OC_LOG_STORAGE_V2.md)は設計段階。長期ログの完成を、`help`などの最初の案内Commandの条件にはしない。

進め方は、基盤レビュー → 重大な指摘の修正 → 認証・Runtimeの復元経路と少数OCの実LINE確認 → 軽量Commandの移植を基本にする。表示や旧仕様の調査は並行して進められる。

移植の最初は`help`を候補にする。未移植Commandを利用可能として一覧へ出さない。`intro`は旧版の環境情報表示を確認して共通の環境Snapshotへ接続する。`id`は基本表示から検討し、旧版のメンバー検索・管理権限・履歴補完まで一括で「軽量」と扱わない。外部HTTP、検索、通知設定、削除・モデレーションには、それぞれ共通実行部・保存・権限の前提を揃える。

## 別のGPTへ渡す依頼文

```text
上記リポジトリのcommit 6027cfca04bc21217ca7d6eb57fce7943b8b0fd9について、
コマンド移植前の基盤コードを読み取り専用でレビューしてください。
AGENTS.mdとdocs/engineering/FOUNDATION_REVIEW.mdの重点を参照してください。
基盤コードは指定commit、レビュー依頼資料は最新branchを参照できます。

目的は、取りこぼし、通知停滞、API制限時の連鎖、資源上限、終了・復旧の
実際の問題と、コマンド追加前に直す必要がある箇所を見つけることです。
重大度順に、ファイル・行、具体的な発生条件、影響、最小の修正案を示してください。
確定した不具合、仮説、既知の未実装、実LINEでしか確認できない点を区別してください。
実運用可能とは、オフラインSmokeの成功だけで判定しないでください。
不要な抽象化・大量のテスト・大規模リファクタリングの提案は避けてください。
実アカウントへのログイン・送信・設定変更、認証値や実ログの読取は行わないでください。

最後に、help / intro / idの基本表示をどこまで先に移植できるか、
重いCommandや管理Commandの前に必要な最小作業を整理してください。
```

実行確認は既存のbuild・Smokeを使う。依存やNative compilerが利用できない場合はその制約を報告し、実行していない結果を成功として書かない。
