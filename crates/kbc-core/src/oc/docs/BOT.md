# OCでのBotの表示名変更

2026-10-04。利用者指定でBOT管理者専用。実装とオフライン検証の対象。実LINEの表示変更は配備後の利用者試験で確認する。

## 入力・対象・権限

`!bot name 名前` と `o.bot name 名前`。実行したトークが属するOCのBot自身のプロフィールを変更する。OC名やアカウント全体の名前は変更しない。別OCでは別のプロフィールを持ち、同じOCのサブトークは共通のプロフィールを使う。[LINE公式のプロフィール説明](https://help.line.me/line/smartphone?contentId=20005375&lang=ja)、[サブトークの説明](https://help.line.me/line/android/sp?contentId=20024745&lang=ja)。

現在のOC所属を確認後、共通bot_rankでBOT admin以上だけを許可する。OCのADMIN / CO_ADMINやBOT modだけでは実行できない。権限はCore SQLiteのbot_rolesを参照し、機密permissionsファイルは初回移行だけに使用する。BotがOCの一般メンバーでも、自分のプロフィール更新をAPIへ依頼できる。サーバー側で拒否された場合は結果を報告する。

利用者の追加指定で20 UTF-16単位・改行・制御文字の独自制限を撤廃した。name直後の区切り用ASCIIスペース1個だけを除き、残りの文字列を前後の空白・改行・制御文字も含めてそのままAPIへ渡す。引数なしの空文字だけは使い方案内とする。LINE側が拒否・正規化する可能性があり、すべての名前が使えることは保証しない。入力8KiB・continuation 48KiB・OC結果32KiBなど既存の共通資源上限は維持する。結果表示だけは80文字のプレビュー・制御文字の可視化を使い、実際の改名文字列は短縮しない。`!bot` / `!help bot` / `!help bot` は [共通txt案内](../../../../../content/help/bot.txt) を返す。

## 関数と既存基盤の関係

| 関数 | 役割・後続 |
| --- | --- |
| bot::parse | prefixとname引数を識別し、名前本文をInputへ保存 |
| oc::ingest / complete | 既存Context照会で実行者と所属を確認し、bot::executeへ渡す |
| bot::execute | BOT管理者と入力を確認し、Member照会へ進む |
| bot::target | Bot自身・OC・JOINED・未加工の現在名を確認。完全一致なら更新しない。再度BOT権限を確認し、Profile変更Actionを発行 |
| SquareDirectory.execute(member / profile) | Member照会のrawMemberNameで未加工の現在名を渡す。Profile変更は実行トークの親OCとBot MIDを照合し、updateSquareMemberへ渡す。その後にgetSquareMemberを1回行い、MID・OC・未加工の名前・JOINEDを確認して成功を返す |
| bot::mutation | 共通oc_historyへ旧名・新名・結果を記録し、実行トークへ通常送信で報告 |
| bot::display_name | 結果メッセージ用に80文字へ短縮し制御文字を可視化。APIへ渡す本文には使わない |

Protocol v9のProfileは変更要求。v10はOcResultに任意のrawMemberNameを追加し、表示用DTOの改行除去・80文字短縮による同名誤判定を防ぐ。Profileの成功確認もSDKのdisplayNameそのものと比較する。LINEJS 3.4.2のupdateSquareMemberでupdatedAttrsはDISPLAY_NAME（数値1）だけ、updatedPreferenceAttrsは空。Bot MID・OC MID・現在revision・新しいdisplayNameを渡し、role・membershipStateを更新しない。旧Botのsrc/runtime/ocProfileStatus.tsが使用していたAPIを参照した。旧版の自動状態suffixや定期巡回は移植しない。2026-10-04にnpmの最新公開版3.4.2を再確認し、依存lockの変更は不要だった。

専用Queue・Cache・Table・定期取得は追加しない。既存のOC Outbox、照会Worker、有限配送と共通API制御を共有する。受付・権限確認の60秒期限、変更配送の30秒期限、fetch直前のsending記録を引き継ぐ。通信後の不明結果はunknownとして残し、自動再更新しない。再起動後もLINE側のプロフィールが正とし、古い保存名を再適用しない。

## 検証範囲

smoke:ocはOC管理人・副官・BOT mod・一般参加者の拒否、BOT管理者の成功、サブトークの親OC解決、日本語・空白・絵文字、同名の更新省略、roleとmembershipStateの非変更、通信後unknownの非再実行を検証する。制限撤廃後は80文字を超える名前・前後の空白・CR / LF・TAB・NUL・ESC・U+0085を混ぜた入力がAPIへ完全一致で渡り、成功判定されること、再入力時の同名更新省略、表示整形後の文字列へ実際に改名できることを同じシナリオで確認する。API契約はモック検証であり、実LINEでの改名成功・禁止文字の挙動とは区別する。

2026-10-04の制限撤廃後にnpm run build、smoke:oc、smoke:commands、既存smoke、cargo clippy --workspace --all-targets --release --locked -- -D warnings、BOM / LF / 資料リンク検査を通過した。

初回のv10配備後に実LINEでupdateSquareMemberが呼ばれ、API errors 0のままInvalidSquareMemberResponseでunknownとなる配送を2件観測した。更新応答をgetSquareMemberと同じ完全なDTOとして扱っていたため、Profile更新後にgetSquareMemberを1回行う確認へ修正した。更新要求は再実行せず、読み取りが失敗・所属や名前が不一致ならunknownを維持する。追加取得は手動改名時だけで、定期処理には追加しない。Smokeの更新応答を空のオブジェクトとして、この返答形でも読み取りで成功確認できることを検証する。

## BOT権限・稼働管理の移植（2026-10-06）

旧LINEの `src/commands/bot.ts`、`permissionArgs.ts`、`permissions/store.ts` と `main.ts` の停止判定を参照した。通常の管理機能とstatusを移植し、利用者指定によりstatus test / envは後回し。従来のname入力と未加工プロフィール変更を維持する。

### 入力と権限

- `!bot setting admin|mod @対象` または `userID:<pMID>` / 裸のpMIDで登録・更新。`del`で指定した役割だけ解除する。一度の操作は旧版どおり一人。例: `!bot setting mod userID:<pMID> del`。同一人のadminからmodへの変更は降格となる。
- 既定は実行OCのsMID。権限判定・既定一覧はsMIDと実行トークmMIDの高い権限を参照し、既定の登録時はその二つの登録をsMIDへ統合する。別のサブトークにあるmMID登録は変更しない。明示した `talkID:<sMIDまたはmMID>` はそのscopeだけを設定・解除する。OC管理人は実行OCまたは実行トークだけ、現在のBOT管理者は遠隔設定もできる。副官・BOT mod・一般参加者は委任できない。遠隔で未参加OCへ登録してもLINE権限や参加状態は変わらない。
- `!bot admin [talkID:<sMIDまたはmMID>] [3p]` は一般参加者も閲覧可能。1ページ10人。選択用Sessionではなくコマンド引数でページを指定する。観測済みの名前とMIDを表示し、一覧の全員へLINE照会を行わない。名前未取得でもMIDを表示する。
- `!bot setting status [talkID:<sMIDまたはmMID>]` は本人のBOT権限と実行OCの役割を表示する。旧banコマンド・userBans / talkBansは今回の対象外で、BAN状態を実装済みとして表示しない。
- `!bot stop` / `start` はBOT管理者・OC管理人・副官。`all`はBOT管理者のみ。個別はmMID単位でサブトークを止めない。全体停止と個別停止は独立し、start allでも個別停止は残る。自分自身の権限変更も旧版同様許可するため、操作後に権限を失うことがある。

旧版同様、停止中はstartだけ受け付ける。受信・checkpoint・長期ログは継続するが、新しいCommand・参加退出通知・自動処分は受付を止め、結果待ちの読み取りから新しい操作を発行しない。停止設定を書いたコマンドの確認返信は送る。処分や送信の結果確定・unknown照合は維持し、停止前から配送待ちのActionを取消・再送しない。停止先へ新しいOC通知を生成しない。ストア監視・スケジュール検知は続け、停止先には新規通知を登録しない。停止中に見送った更新を再開時にまとめて送らない。停止中のOC短期メンバー状態は更新されず、再開後の新着イベントで更新する。

### statusと保存

`!bot status`は稼働・全体/個別停止設定・時間分秒の稼働時間・本人のBOT権限・最終イベント受付・照会/配送/素材/予定時刻待ち・結果不明・未同期ログを表示する。2026-10-09のProtocol v22からAdapterのCPU・メモリ・API枠・受信状態・ビルド情報も表示する。停止・ミュート中も受け付け、LINEへのContext照会を行わない。本人のBOT権限はSQLite、OC権限は未取得と表示する。現在のOC権限は!bot setting statusで確認する。資源計測は既存60秒metrics Timerを使い、専用API・Timer・Workerは追加しない。[値の意味・関数・上限・検証](../../../../../apps/line/docs/RUNTIME_STATUS.md)。

permissions.jsonのSQUARE admin/mod、OC個別botStops、globalBotStopをCore起動時のtransactionで初回だけ取り込む。重複権限はadmin優先。実際のMIDはコード・公開repoへ埋め込まない。`bot_permissions_imported`で再取り込みを防ぎ、変更はSQLiteのbot_roles / bot_stopsを正本にする。毎分の既存暗号化GitHub snapshotに含める。旧settings/permissions.jsonへの書き戻しは行わず、旧版と新版はそれぞれ別の正本を持つ。再起動しても解除済みの権限を旧ファイルから復活させない。ローカルDB消失時は最後の成功したsnapshotから復元し、退避後の変更が失われ得る契約は従来どおり。

権限4096件・停止2048件・取込ファイル256KiB。共通DBの64MiB・Outbox2048件・入力8192byte・60秒権限照会期限を共有する。権限変更と応答・操作履歴は同じtransactionで確定し、失敗時はまとめてrollbackする。

| 関数 | 働きと接続 |
| --- | --- |
| permissions::initialize | table作成・初回ファイル取込・移行markerを原子的に保存 |
| permissions::rank / set_role | SQLiteで最新権限を照会・更新。OC処分保護・Test・pushsetting・nameも同じ判定を使う |
| permissions::stop_state / stopped / control | 個別・全体停止を保持し、受付・通知登録・スケジュール配送へ接続 |
| bot::parse | name本文を保持しつつ、他のbot subcommandを共通Context照会へ渡す |
| bot_management::execute / grant / list | 現役割とBOT権限を照合し、設定・状態表示・有限な一覧を既存返信へ渡す |
| oc::ingest / complete / text_action | 停止中の新しい処理・通知を抑え、startと停止/再開の確認返信、送信結果の確定を維持 |
| Runtime::stats_from_db | statsとstatusで共通の現在値を使用し、transaction中の再lockを避ける |

利用者向け定型文面はmessages/bot.txtとschema、入力案内はhelp/bot.txt。!helpのindexは既存bot項目を維持し、自動の一覧追加はしない。実LINEでの動作確認と本番配備はこの作業では行っていない。

一覧の観測名参照はOC/MIDの主キーで絞り、OCメンバー全件を候補ごとに走査しない。権限の最新判定はscope/memberの主キーを使うSQL一回となる。常駐HashMapよりSQL呼出が増えるが、変更・判定・応答の同じtransactionと再起動復元を優先する。停止した副官部屋へ審議promptを生成できない場合は、promptのない審議を保存しない。2026-10-06にnpm公開版3.4.2を再確認し、依存lockは変更していない。

検証（2026-10-06）: build・TypeScript check・Clippy（workspace/all-targets/release/警告拒否）とRustテスト8件、OC・Command・文面・Persistence・SKD・受信配送・ログSmokeが通過した。OC Smokeで委任権限、遠隔操作、admin/modの降格と役割別解除、mMID登録のsMIDへの統合、解除後の再起動、status、個別/全体停止とstart allの独立性、停止中のログ継続を確認した。Rustの既存監視テストに、停止中の更新見送り・検知位置の保存・再開時の非再送と容量保持を加えた。実LINE APIへ変更・通知は送っていない。

2026-10-08、bot_management::quick_statusはstatus単独を受信時に判定し、statusへ渡す。statusはstats_from_db・stop_state・rankを同じtransactionで読み、実行元へのtext_actionを登録する。通常のexecuteからも同じstatusを呼び、旧保存済みContext Jobに対応する。照会の滞留時も!pingと同じ通常配送経路へ応答を登録できる。送信API自体の障害・配送詰まりを回避できる保証ではない。オフライン検証では、未完了のContext照会を残した状態のo.bot statusと、個別停止中の!bot statusが追加LINE照会なしで応答した。
