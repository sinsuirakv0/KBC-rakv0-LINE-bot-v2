# KBC LINE Bot V2 開発ルール

## 作業前に読む

- `docs/plans/LINE_CORE_V2.md`
- `docs/architecture/CORE_AND_ADAPTER.md`
- `docs/research/LEGACY_FINDINGS.md`
- `docs/engineering/DOCUMENTATION.md`
- 受信・常時処理の変更時は `docs/research/RECEIVER_AND_BACKGROUND_EXPERIMENTS.md`
- 受信・並列制御の変更時は `docs/decisions/PUSH_AND_BOUNDED_CONCURRENCY_V1.md`
- Runtime変更時は `crates/kbc-core/docs/RUNTIME.md`
- Adapter変更時は `apps/line/docs/ADAPTER.md`
- Command変更時は `crates/kbc-core/src/commands/docs/COMMANDS.md` と `content/docs/CONTENT.md`
- 起動・配備の変更時は `docs/operations/MINIMAL_BOT.md`
- ログ保存・同期の変更時は `docs/decisions/OC_LOG_STORAGE_V2.md`

## 基本方針

- 構成と設計原則はDiscord Bot v2、コマンドの仕様は旧LINE Botを参照する。
- 第一段階はOpenChat専用。参加OCは原則利用可能とし、個人・グループの許可設定は後続段階で扱う。
- 新Botのコマンドprefixは `o.` 。例: `o.ping` 。旧版の `!` を新Botの既定値へ持ち込まない。
- 第一段階は、多OCでの受信取りこぼし防止、通知の自律配送、全APIの負荷制御を最優先にする。
- 基本の受信方式はOC PUSH。同じcursorの取得は直列、独立したトークの取得・Command・配送は有限並列にする。補助取得は不足する情報と必要なトークへ限定する。
- BotロジックはRust Coreへ置き、TypeScriptはLINEJSの通信・認証・入出力を扱うAdapterとする。
- LINEJSは最新公開版を使う。実装開始・依存更新時に公開版を再確認し、採用versionとlockを記録する。旧版用の独自パッチは採用版で必要性を確認してから扱う。
- LINEJS / Node.jsのオブジェクトをRust Coreへ渡さず、version付きProtocolを使う。
- 変数名・関数名は英語、コメントは日本語。
- 日本語を含む `.ts` / `.md` / `.env.example` 等はBOM付きUTF-8で保存する。
- コードはシンプルにし、生成後に書きすぎていないか確認する。
- 共通化できる処理は再利用するが、利用者のいない抽象化は作らない。
- Queue、Cache、Session、Task、Buffer、並列数は有限にする。
- CommandごとのHTTP・Cache・Queue・Timeout・Progress基盤を作らない。
- 性能改善は応答時間の内訳と常時CPUの計測に基づく。
- テストは最小限。型・Compilerが保証することを重複してテストしない。
- 要件が曖昧な場合は質問し、推測で大きな変更をしない。
- より良い設計を発見した場合は、実装前に理由とトレードオフを示す。
- 各機能フォルダの `docs/` に、仕様、関数の働きと相互関係、実験結果を残す。
- Discord Bot v2と同様、requirements / architecture / decisions / implementation / research / operations / plansを役割別に使い、コード変更と同じ作業で関連資料を更新する。
- 実装済みの挙動と提案・実験結果を区別する。常時処理や受信方式はLINEJSの採用版の実装を確認し、比較が必要な場合だけ小さな実験を行って採否を残す。
- 実装Phaseの開始前に対象の旧コードを読み、古い資料の記述だけで仕様を決めない。

## 運用・Git

- 自動実行OK: 全て。
- 目的ごとにcommitし、大量のファイルを1commitにまとめない。
- commit messageは日本語。
