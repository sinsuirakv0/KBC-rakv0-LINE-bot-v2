# 予約通知・イベント通知

2026-10-08。旧LINE Botの `src/commands/push.ts`、`src/reminders/`、`src/eventPush/` の現行コードを参照する。Coreに実装。実LINE・本番の長期運用は未確認。

## 入力と登録

`!` と `o.` の両方に対応。送信先は実行したOpenChatのトーク。旧版と同じくOC参加者が設定でき、BOT管理者限定にはしない。停止・mute等は既存のOC入口を通る。

| 入力 | 動作 |
| --- | --- |
| `!push 10 [本文]` | 10分後の予約。本文なしでも登録する |
| `!push 10/9 [本文]` | 今年10月9日の0時JST。過去なら拒否 |
| `!push 2026/10/9-12:00 [本文]` | 年・日時指定。別引数の時刻、括弧の時刻も旧版どおり受け付ける |
| `!push event` | 設定案内。個別登録時にトークの登録も行うので、先行する空の登録は不要 |
| `!push event ID・名前 [-5]` | 個別イベントの開始・終了10分前。`-5` なら開始5分前も追加 |
| `!push event g [ID・名前] [-5]` | ガチャの開始・終了10分前。対象省略ならR/E/N全ガチャ |
| `!push event all` | 旧版の全イベント開始通知。終了前通知は含めない |
| `!push event daily` | 毎日22時JSTに翌日24時間のイベント予定をスレッドへ送る |
| `!push event 対象 del` / `!push event g [対象] del` | 個別・全件・dailyの解除。名前の候補選択からの解除も対応 |
| `!push event del` | このトークのイベント・ガチャ・dailyを全解除。時間指定の予約は残す |
| `!push status [3p]` | 設定10件ずつと未完了の予約件数。更新通知の状態は従来のpushsettingへ分離 |

日時はJST、数字は分。0以下、存在しない日付・時刻、過去、10年より先は拒否する。本文省略時は文面キーの既定文。通知時は登録者をメンションするが、コマンドの確認応答は通常投稿。予約本文は旧版と同様に引数を空白で結合する。

イベントIDは0〜2,147,483,647。現在の `sale.json` に未掲載でも登録でき、後からそのIDの予定が掲載されたときに通知する。名前指定は通知対象の解決に使い、参照用の検索コマンドにはしない。1候補は即登録、複数候補だけ番号選択する。ガチャの番号指定はガチャIDで、名前候補はR/E/N別のガチャID。数字で登録すると同IDの各種類を対象にし、名前で選択すると候補の種類にも限定する。シリーズIDを登録する機能は今回含めない。

個別イベントでは同じ予定内の102/112を優先する旧制限を使わず、指定IDそのものを見る。全件・dailyでは旧版どおり102/112の優先表示、ミッション等の除外を維持する。`maxVersion`が現在版より古い予定は通知しない。終了前通知は開催開始より前になる短い予定では送らない。

## 候補と文面

既存Sessionを共有し、同じトーク・送信者・送信成功した最新一覧だけを操作できる。1ページ10件、数字は選択、「次」「前」「3p」はページ移動、「終了」「取消」は終了。期限は送信成功から10分。次ページが送信されるまでは旧ページを維持し、移動中の番号入力で設定しない。成功後に旧一覧を管理者削除する。失敗・unknownでは既存の復旧契約を使う。

候補は最大512件。保存するのは種類・ID・短い表示名・ページ・事前通知分数・登録／解除の操作。公開データ全体はSessionへ保存しない。表示名は最大24 UTF-16単位へ短縮し、通知の正式名は監視時の最新データから引く。Session全体128件・payload 64KiBを共有し、開始不能時はその旨を返す。sale参照コマンドの30秒・最大9件選択は維持する。

利用者向け文面は [push.txt](../../../../../content/messages/push.txt)、公開案内は [help/push.txt](../../../../../content/help/push.txt)。help/indexへの登録は手動で行う。

## 関数と相互関係

```text
commands::prepare → CommandPlan::Push → sessions::apply → push::apply
  時間予約 → 未来dueの既存Outbox → next_actionのTimer → 通常配送
  ID・all・daily → configure → SQLite設定
  名前 → PrepareMedia(EventData push) → 共通準備Worker → lookup
       → finish_event_data → 1件ならconfigure / 複数なら既存Session
       → リプライ → select → configure または次ページ
run_store_monitors → push_loop → 共有Source → collect → dispatch_push/save
       → 同じSQLite transactionで送信予定・重複印・走査位置を確定
       → 通常配送 / 親送信成功 → skd::complete → 1秒後のスレッド本文
```

| 関数 | 働き・依存 |
| --- | --- |
| `time::parse` | 旧入力形式を解釈し、カレンダー・期限・省略本文を検査 |
| `apply / configure` | 受付transaction内で予約・設定・解除・応答を保存。HTTPしない |
| `lookup / render_page / select` | 共通Sourceによる名前解決、共通paginationによる一覧、送信成功前後のページ整合 |
| `schedule::periods` | 共通ScheduleHeader / TimeBlockから月日・曜日・跨日・年跨ぎの開催を展開 |
| `push_loop` | 共通監視の寿命・取消を共有し、公開データを更新する |
| `collect / daily` | 個別・全件の重複をまとめ、開始・事前・終了・翌日予定を生成 |
| `dispatch_push / save` | Outbox容量・履歴を確認し、有限に予定を配送へ渡す。満杯時は走査位置を進めない |
| `next_action_mode` | push通知を取り出す時点でも停止を再確認。通信前に停止なら確定失敗として送らない |

独自Client・LINE API Queue・配送Worker・バックアップWorkerは追加しない。Protocol v18とAdapterは変更しない。[設計判断と上限](../../../../../docs/decisions/PUSH_NOTIFICATIONS_V1.md)、[Runtime](../../../docs/RUNTIME.md)を参照。

## 保存と復旧

`push_subscriptions(chat,kind,target,advance,since)`、`push_destinations(chat,checked)`、`push_deliveries(id,chat,due)`を既存SQLite snapshotへ含める。未来の予約はactions自体へ保存する。設定変更時には該当トークのまだ未来にある未送信イベント予定を再計画する。既に期限を迎えた予定、通信中、unknownをこの再計画で勝手に再送しない。全解除は未送信のイベントActionも除き、解除済みdailyの親送信が完了しても新しい本文を登録しない。

通常の設定変更では、親が送信済みのスレッド本文を再計画対象から外す。別イベントの設定変更で1秒待機中のdaily本文を消さない。daily自身を解除するときだけ、その未送信本文も取消す。

通常再起動は未完了の未来予定を維持し、通信開始済みはunknownへ変える。unknownは既存運用操作で照合する。停止中のイベント走査位置は現在まで進め、再開時に停止期間分をまとめて通知しない。既に作成されたpushの未送信通知も、取り出し時に停止なら抑制する。

予約の未解決メッセージは最大1,024件。長期予約だけで共通Outboxを埋めない。予約のevent_idは元の受信IDで、Action IDのprefixで通知種別を識別する。未完了予約が残る間は元の受信IDを清掃しないため、48時間を過ぎたコマンドの再取得でも重複登録しない。

旧JSONの通知設定・予約の自動インポートは今回追加しない。GitHub退避の間隔と復元時の不確実性は既存の[保存契約](../../../../../docs/operations/GITHUB_RECOVERY.md)に従う。通知の完全なexactly-once、公開データから消えた過去予定の復元、多OCのAPI制限回避を保証するものではない。

## 検証

Coreの最小回帰3件で、日時・省略本文、再起動・所有者・ページの確定前後・未掲載ID・自律通知・unknown・停止、時間帯・重複・ガチャ・容量・dailyと本文の待機を確認する。公開データの単発確認は通常テストではignoreし、LINE通信は行わない。

2026-10-08、Native経由で予約受付・受信なしの期限配送・登録者メンション・未掲載ID・o.別prefix・ガチャ全件・daily・status/help・実際の公開名候補からの番号登録・Session開始不能時の案内・予約1,024件上限を確認した。既存smoke / commands / oc / messages / persistence / skd / logsも通過。LINE送信は0件。公開カタログの単発確認は15通知を生成し、候補とスレッド本文の上限を確認した。

`npm run build`、`npm run check`、`cargo clippy --workspace --all-targets --release --locked -- -D warnings`、fmt、文面391キー／462呼出の整合、BOM・資料リンクの確認は通過。`cargo test -p kbc-core --release --locked` は13件成功・4件ignore（公開通信の単発試験）。設定変更で送信済み親の本文を維持し、daily解除でその未送信本文を取り消す回帰も通過。本番配備は行っていない。
