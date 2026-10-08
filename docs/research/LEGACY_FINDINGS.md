# 旧LINE Botの初回調査と引き継ぐ知見

調査日: 2026-10-01（JST）
旧LINE参照: `D:/KBC/KBC-rakv0-line-bot` 、HEAD `c6796e0` 。
Discord設計参照: `D:/KBC/KBC-rakv0-discord-bot-v2` 、HEAD `02e6e9b` 。

資料・コードの静的調査。現在の本番測定、LINEログイン、API probeは行っていない。旧LINEには未追跡の `_codex_git_stage_20260912/` があり、今回の参照対象から除外した。

## 1. 現行コードで確認したこと

| 項目 | 確認結果 | 主な参照先 |
| --- | --- | --- |
| 対象 | `enableTalk: false` 固定。現在はOC専用 | `src/config.ts` 、 `docs/architecture/OC_ONLY_RUNTIME.md` |
| LINEJS版 | package指定は `^3.1.4` 、lockと手元node_modulesは `3.1.4` | `package.json` 、 `package-lock.json` |
| Command | Registryの18 Commandと別Handlerの `ping` | `src/commands/index.ts` 、 `src/main.ts: dispatchText` |
| Command実行枠 | 既定同時2件、待機30件 | `src/config.ts` 、 `src/runtime/workload.ts` |
| API実行 | 既定同時2件、待機上限100、50ms間隔、critical / high / normal | `src/config.ts` 、 `src/runtime/lineApiQueue.ts` |
| 優先枠 | 待機critical 10件・high 20件、critical実行予約1件 | 同上 |
| 制限アカウント | 新規APIを停止。再接続待機は5分固定 | `src/config.ts` 、 `docs/operations/LINE_ACCOUNT_RESTRICTION_RECOVERY.md` |
| 履歴読込 | `MESSAGE_LOG_READ_ENABLED=false` が既定。通常検索は既定7日 | `src/config.ts` 、 `src/messageLog/store.ts` |
| メモリ判定 | cgroup currentからinactive fileを除くworking setを使用。既定90%で制御終了 | `src/runtime/memoryGuard.ts` |
| 認証とカーソル | Square同期トークンの所有MIDを検査。所有不明・変更時はカーソルを破棄 | `src/storage/lineStorage.ts` |
| 場所と権限 | OCの権限scopeとトーク停止等は異なるID単位を使う | `src/permissions/store.ts` |

これらは旧版の現在値であり、新版の上限を確定するものではない。最新要件は0.2コア・512MB、第一段階はOpenChat専用・参加OCを原則許可。個人・グループの許可設定は後続段階でOC内から行う。

## 2. 過去の運用知見と新版への反映

| 問題・観察 | 引き継ぐ判断 | 参照 |
| --- | --- | --- |
| HTTP Serverだけ生存し、LINE受信が止まる | livenessとReceiverのreadinessを分け、無通信時も受信APIの正常終了を観測する | `docs/operations/LINE_RECEIVER_HEALTH.md` |
| OC操作のNOT_AUTHORIZEDを認証切れと誤認する | 操作権限エラーと認証を分離。必要な認証確認後だけ全体再ログイン | `docs/operations/LINE_SESSION_RECOVERY.md` 、 `src/main.ts: runSession` |
| 単一ReceiverやGitHub障害が全体停止へ波及する | 局所復旧と背景処理のエラー境界を維持する | `src/runtime/receiverSupervisor.ts` 、 `sessionManager.ts` |
| V3認証・SYNC4で約110秒待機、PUSH pingが来ても本文未受信 | 接続成功やpingだけを購読成功とみなさない。個人受信を現在の版・端末で再検証 | `docs/operations/LINE_RECEIVER_HEALTH.md` の2026-07-29実測 |
| LEGYのAbortSignal欠落、PUSH readiness / abortの問題 | 採用版で修正済みか確認。必要なパッチだけを固定・検証し、未知形式は起動失敗にする | `scripts/patch-linejs-legy-signal.mjs` |
| 無効Square syncTokenの再使用でOCが停止する | 無効カーソルだけを初期化。認証や設定を一括削除しない | `src/main.ts: listenRawSquareEvents` 、 `src/storage/lineStorage.ts` |
| アカウント変更後に旧カーソルや未参加OCへアクセスする | カーソル所有者とアクセス可否を分離。設定を保持し、未参加トークを休止する | `lineStorage.ts` 、 `src/runtime/docs/square_chat_access.md` |
| 1トークの遅いAPIで他トークの返信も詰まる | 有限な宛先別実行、優先枠、最長実行時間を観測する | `docs/architecture/LINE_API_QUEUE_DESIGN.md` 、 `lineApiQueue.ts` |
| 大容量JSON、Git Trees全走査、全履歴索引の常駐でOOM | 起動で全件走査しない。対象トーク・期間を絞り、パート単位で読む | `docs/storage/MESSAGE_LOG_STORAGE.md` 、 `src/messageLog/docs/WRITE_ONLY_MODE.md` |
| GitHub SHA競合、失敗・大量同期で受信や返信が遅れる | 共通の直列mutation、有限バッチ、失敗分を保持。ログpartの成功後にmanifest | `docs/storage/GITHUB_SYNC_DESIGN.md` 、 `src/storage/githubContents.ts` |
| 高い常時CPUと長い応答時間 | 常時ポーリング・巡回・JSON処理と送信待ちを別々に測る | 利用者の今回の運用報告、 `docs/operations/BOT_PERFORMANCE_NOTES.md` |

Talkに関する約110秒やPUSHの観察は過去の特定環境の結果であり、現在のLINEJSでも同じとは断定しない。

## 3. 今回の障害報告と静的調査

| 利用者の報告 | 確認した実装 | 次に調べる点 |
| --- | --- | --- |
| Command後に通知が止まり、別Commandで送られる | API Dispatcherに完了後Timerがあり、Event schedulerにもintervalとrerunRequestedがある | 根拠なく「Timerがない」と決めない。起床・実行中・依存通信・枠・API制限のどこで止まるか追跡 |
| 2件以上をほぼ同時送信すると片方が無反応 | `handleRawSquareEvent` は `void handleSquareMessage(...)` で処理を開始。取得側は完了を待たず同期token保存へ進める | 取得ページの全件、受付、処理、送信結果をmessage IDで突合。未完了とcheckpointの整合性を検証 |
| API制限を時々受ける | `fetchMyEvents` 、名前・トーク照会等は送信・削除用の `lineApiQueue` の外にも存在 | 全API件数とエラーcode / scope、SDKの制御を観測。送信間隔だけをAPI負荷予算としない |

追加で確認した経路:

- `listenRawSquareEvents` は `fetchMyEvents(limit: 100)` を呼び、ページ継続時25ms、通常1秒待つ。実通信時間・空取得率は未測定。
- `createdAt < sessionStartedAt` のイベントは旧イベントとして扱い、メッセージCommandは実行しない。再起動・再接続前に作られた未処理入力も該当し得るため、checkpointと処理済み判定を見直す。
- `handleSquareMessage` は自身判定、scope解決、moderationを経てCommandへ進む。無反応を取得漏れだけと断定せず、途中の判定・照会・待機も確認する。
- `lineApiQueue` の50msはscopeの実行可能時刻に適用される。アカウント全体の全API件数上限ではない。
- `RuntimeWorkload.runBackground` の背景Queueには、この実装内で受付件数の上限がない。新版へそのまま移さない。
- `EventPushScheduler` はcheckをawaitしてrunningを保持する。依存通信が終了しなければintervalによるrerunRequestedだけでは進めない。
- `fixedOcPollingDecision` と `src/config.ts` は参加退出補助監視を2秒固定とする。OC数に応じた呼出数を調べる。

上記は実コードから分かる構造・危険箇所である。通知停滞や同時入力の欠落の実原因は再現・ログ突合前には確定しない。旧版の修正・API実行は今回行っていない。

## 4. 資料の差異

| 古い記述 | 現行コード・後続資料 | 扱い |
| --- | --- | --- |
| Talkも運用対象 | `enableTalk: false` 固定 | 第一段階はOC専用を維持。個人・グループは後で検証 |
| 制限時6時間待機 | 5分固定 | 原因・現在仕様を確認して移す |
| LINE API全体1並列、high / normal | 宛先別処理、2並列、critical追加 | 古い値をコピーしない |
| OC補助監視の省エネ・活動量制御 | OC専用資料は常時2秒。省エネを使わない | CPU改善候補。検知遅延を評価してから変更 |
| 通常ログ検索30日 | 現行既定7日 | 現在の設定と表示を基準にする |
| Git Treesで全索引を復元 | 後続資料ではOOM対策で廃止 | 旧案を再導入しない |
| Memory全体約2.2GiB、1コア | 後続資料の実割当は約0.1コア・244MiB | ホスト表示とcgroup割当を分ける |

読み込んだ主な基盤資料は `CORE_RUNTIME_ARCHITECTURE.md` 、 `CODEBASE_DESIGN_REVIEW.md` 、各operations資料。文書内にも「完了」と「次段階」が重複する箇所があるため、実装との照合を優先する。

## 5. 関数の働きと移植先の関係

| 旧関数・型 | 働き・呼出関係 | 新版での配置案 |
| --- | --- | --- |
| `main → SessionManager.run → createLineClient / runSession` | 認証、接続寿命、再接続、受信器の組立 | TS Adapterの接続管理 |
| `ReceiverSupervisor.run / restart` | 子AbortControllerで受信器だけを停止・再作成 | TS Adapter。Talk / Squareで独立 |
| `listenRawSquareEvents` | 取得、ページ処理、カーソル保存、無効token復旧 | TS受信層とRust Inboxへ分ける。Domain判断はCore |
| `dispatchText → handlePing / handleLineCommand` | 権限、実行枠、進捗、Command実行が現在TSに混在 | RustのAccessPolicy / CommandRuntimeへ移す |
| `LineCommand.execute → ReplyableLineMessage.client` | CommandからSDKに直接照会・操作できる | DTO入力と必要な照会・操作Actionに分割 |
| `LineApiQueue.run / pause / resume` | 優先度・scopeで実行。制限時の新規抑止と待機取消 | 共通DispatcherとCoreの配送Policy |
| `ensureSquareSyncOwner → Square token削除・所有MID保存` | 所有者不明・変更時にSquareカーソルのみ破棄 | TSの認証・接続管理とRust checkpointの所有者検査 |
| `startMemoryGuard → readMemoryPressureSnapshot` | cgroup working setを測り、単一の制御終了を要求 | TS環境観測・Lifecycle |
| `messageLogStore.flush → remoteSync → githubContentsClient.write` | ローカル保存と遠隔同期を分け、partを先に送る | Rust Storage。形式と復旧順は要調査 |

この表は移植の境界を示すものであり、各関数をそのまま新設する指定ではない。対象機能の実装時に、そのフォルダの `docs/` へ詳細を残す。

## 6. 外部確認と未調査範囲

[LINEJSの開発元リポジトリ](https://github.com/evex-dev/linejs)と[開発元ドキュメント](https://linejs.evex.land/)を2026-10-01に確認した。開発元はNode.jsとTypeScriptの対応を案内している。[メッセージ受信資料](https://linejs.evex.land/docs/message-event)は `square:message` 、[Client Options](https://linejs.evex.land/docs/client-options)はcustom fetchとStorage拡張を案内する。これらはSDKの機能説明であり、Square APIの実際の制限値や取りこぼしのない配送保証を示す資料ではない。最新公開版は同日の追加確認で `3.4.2` と確定し、新Botで使う方針とした。[改善点と旧パッチの扱い](RECEIVER_AND_BACKGROUND_EXPERIMENTS.md#8-最新公開版の改善と採用方針)を参照する。SDK保護機構の実効性と旧PUSH待機延長の必要性は実行環境での検証が残る。

全subcommand、データschema、保存復元の優先順位、実際のCPU内訳、現在のTalk復号・受信方式は未確定。Phase 0で追加調査する。 `.env` 、認証token、実メッセージ履歴は今回読み込んでいない。

## 7. LINEJS実装と常時処理の追加調査

[受信・常時処理の調査と実験](RECEIVER_AND_BACKGROUND_EXPERIMENTS.md)に、手元SDK 3.1.4と開発元の固定commitの確認、および通信を使わないStream検証を記録した。定期取得とOC PUSH、補助監視、名前巡回、通知・同期の比較が必要な箇所を分類している。

Stream検証は受信経路を選ぶための知見であり、旧Botの無反応の原因を確定するものではない。最新公開版を使う方針は確定し、その後[3.4.2配布物の認証なしProbe](../../experiments/linejs-receiver/docs/RECEIVER_PROBE.md)を実施した。実LINEでの方式比較、API制限の実測、Northflankの採用Runtimeでの検証は未完了。

## 2026-10-08：pushの現行コード確認

旧LINE c6796e0のpush.ts、reminders/time.ts・store.ts・scheduler.ts、eventPush/catalog.ts・schedule.ts・daily.ts・policy.ts・scheduler.tsを確認した。数字は分後、日付はJSTで、旧予約は本文必須。個別IDはsale.json掲載と先行トーク登録が必要で、102/112を含む予定では他のIDを扱わなかった。全イベントは開始のみ、個別は開始・終了10分前・任意の事前通知、dailyは22時に翌日予定をスレッドへ送る。各旧schedulerが直接LINEへ送信し、失敗は別途再試行していた。

新版は利用者指定で本文省略・未掲載ID登録・ガチャ個別と全件を追加し、候補を選ぶ目的を通知設定に限定する。dailyの指定は旧dailyの移植として確認済み。保存・配送は共通Outboxを使う。[確定した仕様と関数](../../crates/kbc-core/src/push/docs/PUSH.md)。


2026-10-08、旧id.tsのsearchMembersはSquareChat.getMembersの一覧と名前フィルタ検索を併用し、名前指定で一致しなければ空displayNameで補完していた。SDK3.4.2のgetMembersもgetSquareChatMembersの継続取得であることを確認。新版の部分名0件報告に対し、無制限な一覧取得を持ち込まず、既存Members要求へ任意chatMembersを追加し、参加者一覧API・旧状態の空displayName補完と状態未取得の保存名を候補として扱う。[修正・上限・確認範囲](../../crates/kbc-core/src/oc/docs/ID.md)。
