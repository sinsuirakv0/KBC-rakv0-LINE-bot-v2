# OC管理コマンドと自動処理

2026-10-03。旧LINE Bot `c6796e0` の実装を参照。実装とオフライン検証まで完了。実LINEの削除・退会・通報API、PUSHの参加通知の網羅性、Northflank負荷は未確認。

## 入力と権限

`!oc help` は公開案内、`!oc adminhelp` は管理案内。設定はOC全体のsquareMid単位、入退室通知はトークのsquareChatMid単位。自動処理の初期値はすべてOFF。明示した旧管理設定は初回だけ変換して引き継ぐ。旧ログの変換はデータリポジトリの別Workflowで行う。

| 操作 | 旧版を維持した権限 |
| --- | --- |
| status / authority | OCの参加者 |
| kick | BOT mod以上。OC管理者という理由だけでは許可しない |
| mute | BOT mod以上、またはOC ADMIN。CO_ADMIN単独は不可 |
| url / media / watch / history | BOT mod以上、またはOC ADMIN / CO_ADMIN |
| setup / modroom / main / join / leave / adminhelp / 審議 | BOT admin、またはOC ADMIN / CO_ADMIN。BOT mod単独は不可 |

現在のOC所属と実行者の役割をSDKで照会してからRustが判断する。既定prefixは `!`、`o.` も受け付ける。Bot管理権限は `BOT_PERMISSIONS_PATH` で指定した機密ファイルの `version: 1`、`roles[].chatType/chatMid/userMid/role` からSQUAREのadmin/modだけを読み、squareMidと現在トークの旧aliasを照合する。ファイル未指定ならBOT権限なし。旧コードの初期管理者MIDは公開ソースへコピーしない。権限ファイルは起動時に読み、更新後は再起動する。旧ファイルのban・停止設定は今回取り込まない。

| 新しい入力 | 動作 |
| --- | --- |
| !oc setup | 送信済みメニューに番号をリプライ。複数は `1 2`、解除は `off 1 2` |
| !oc modroom set / off / test | 現在トークを副官部屋へ指定、解除、送信確認 |
| !oc main / main set / main off | 本OCトークを選択、現在トークを指定、解除 |
| !oc join set [--mention] [--id] 本文 | 参加通知。`<name>`置換、複数行、メンション、旧版と同じ短縮ID |
| !oc leave set [--mention] [--id] 本文 | 退出通知。join / leave単独は現在設定、offで解除 |
| !oc url on / off / list [ページ] | HTTPS許可制削除、5件ずつのルール一覧 |
| !oc url add URL [exact / path / prefix / domain] | 許可範囲を追加。remove 番号 / allで削除 |
| !oc media on / off | 同じOC・送信者の30秒間の画像・動画について7件目以降を管理者削除 |
| !oc watch early / danger / cohort / report on / off | 即抜け、初参加危険語、一斉参加、危険語通報の個別切替 |
| !oc mute @対象 170 / 0:17 / 8/7-0:17 / inf | 数字は分、時刻・日付はJST、infは無期限。offで解除 |
| !oc mute list [ページ] | 20人ずつ表示 |
| !oc kick @対象 理由 | 直接BANNEDへ更新。メンションまたはpで始まるMID、1回8人まで |
| !oc history / kick history | OCごとの直近15件の受付・結果。旧hisも可 |

joinmes / joinmsg / joinmessage、leavemes / leftmes / exitmes系、link / linkurl / adlink、mediadel / mediaburst、kicktestはalias。`!oc url`へのURL直書きは行わず、addで範囲を明示する。期限は単一引数の `YYYY/M/D-H:MM` にも対応する。

setupは1 URL、2 media、3副官部屋、4 early、5 danger、6 cohort、7基本項目一括ON（通報を除く）、8自動処理OFF、9状態、10通報、11本OC選択。OFF指定の7も一括解除する。副官部屋からjoin / leaveを設定すると送信先を8件ずつ番号選択する。取得できなかったトークでは、そのトークから直接設定できる。

対話は本人・トーク・送信済みの最新promptに結び付ける。通常の数字、別人、別トーク、古いpromptには反応しない。設定継続時は新prompt送信成功後に旧promptを管理者削除し、10分後の共通清掃も使う。終了・送信先確定時は設定を閉じ、promptの清掃を前倒しする。削除失敗で保存済み設定や新promptを巻き戻さない。

## 自動処理とPUSH

安価な候補判定に一致した投稿だけ、現在の送信者の所属・役割を照会する。URL・media・危険語・一斉参加はOC ADMIN / CO_ADMIN、BOT管理権限、Bot自身を免除し、役割が不明な場合は処分しない。明示muteは旧版どおり管理者免除より先に判定し、mute保存以後の投稿を削除する。

URLはNFKCで抽出し、同じorigin（scheme・host・port）のHTTPSだけを許可する。prefixはパス境界を含めて照合するため `/docs` は `/docs-evil` を許可しない。1投稿65種類以上のURLは許可判定上限を超えた投稿として削除候補にする。審議への通知は1投稿3URLまで。画像グループのGSEQ / GTOTALがある場合も最初の6件を残す。

参加・退出はSquare Eventのcreate/update member、create/join/leave/update chat memberをplain DTOへ正規化する。OC全体とトーク内の所属を分け、同状態・古い状態を重複通知しない。OC全体の通知先は明示した本OCトーク。参加時刻とmember作成時刻が2分以内で、今回の状態保存に過去参加がない場合だけ初参加とみなす。長期ログと短期の処分判定状態は分け、作成時刻不明の参加者を初参加と推定しない。

- 初参加2分以内の「チート」「代行」はNFKCで検知し、投稿削除とKICK_OUT。reportがONの場合だけSCAM通報を先に要求する。失敗・不明の通報は自動再送しない。
- earlyは、OC全体LEFTまたは設定した本OCトークのLEFTを契機に現在のOC全体LEFTを照会する。初参加5分以内だけBANNED、5〜30分は審議。サブトーク退出をOC全体退会とみなさない。再参加者の即抜けは審議のみ。
- 初参加3人以上が2分以内に参加すると30分監視。危険語・URL・招待らしい投稿を副官部屋へ通知し、一斉参加だけで自動処分しない。

副官部屋のURL審議は1 exact / 2 path / 3 prefix / 4 domain / 5却下。処分審議は再参加禁止 / 無視 / 解除。解除は依頼を記録するだけで、BANNED解除APIは実装していない。審議権限と現在の対象所属を再確認する。kick対象のBot・管理者・副官・BOT管理権限・未知roleは保護する。

常時の参加者一覧巡回、旧polling heartbeat、thread/VOOMのノートURL削除は今回持ち込まない。ノート削除は旧実装でも暫定的で権限免除を確認できないため後続調査とする。probe / identityは利用者指定で後回し。PUSHが必ず全参加・退出を通知する保証はなく、実OCでイベント種別と処分の見逃しを観測する。

## 関数と永続化

| 関数 | 働きと相互関係 |
| --- | --- |
| oc::ingest | mute、管理入力・Session・審議、候補判定の順に既存受付transactionで処理 |
| commands::parse / execute / session_reply / case_reply | 引数・旧権限・設定・本人返信を解決し、requestまたは通常返信を登録 |
| request / complete | OcRequestとJobを既存Outboxへ保存。結果登録と後続Action・設定を同一transactionで確定 |
| commands::target / mutation | 現在の対象MID・OC・役割・revisionを照合し、処分結果を履歴へ記録。複数対象は直列 |
| moderation::candidate / execute | 安価な候補と現在roleを分離。処分候補以外の通常Commandは既存apply_commandへ渡す |
| moderation::member_event / confirm_left | トーク状態・参加回数・初参加・一斉参加を保存し、退会照会後に処分を判断 |
| SquareDirectory.chat / execute | OC対応だけを有限cache、roleは現在照会。SDK返値を検査し、BigInt revisionを文字列DTOへ変換 |
| normalizeEvent / Receiver.accept | テキスト・媒体・参加退出を正規化し、Core commit後にcursorを確定。baselineより古い履歴ではOC追加取得を省く |
| Runtime::next_query_action | 既存actionsから読み取りだけをqueryingへ。Adapterの1照会loopで処理し、同じトークの通常配送枠を使わない |
| deliverAction / ApiScheduler | 2通常配送・1照会のloopが全APIの同じ2並列・250ms間隔・cooldownを共有。更新と通報も実fetch直前にsendingを保存 |

queryingは再起動時queuedへ戻し、読み取りは再実行できる。削除・membership更新・通報のsendingはunknownへ戻し、自動再実行しない。SDK通信後のエラーもunknown。通信前の期限切れはfailed。操作受付から権限照会まで60秒、処分Action作成から通信開始まで30秒の上限を置く。照会の継続は元イベント時刻を優先し、別の管理操作に追い越されにくくする。通常返信は独立して進む。

SQLiteのoc_settings / oc_notifications / oc_sessions / oc_members / oc_presence / oc_media / oc_notices / oc_cases / oc_historyが対応する状態を所有する。既存Core DBのバックアップ・復元単位に含める。GitHub同期は未実装。設定2048OC・通知2048トーク、ルール100・mute100/OC、設定payload192KiB、Session128/10分、候補64、審議256/7日、参加・トーク状態各8192、連投4096/30秒、通知抑制1024、操作履歴2048。既存DBの64MiB・未解決Action2048上限も共有する。状態の上限で古い参加記録を落とした場合は過去参加の網羅を主張しない。

`npm run smoke:oc` は実LINEへ接続せず、旧権限区分・本人Session・照会中ping・再起動・unknown非再送、mute/URL/画像、PUSH正規化・通知・OC全体退会・危険語・一斉参加・URL審議を検証する。既存smokeとsmoke:commandsも実施する。実APIの成功を意味しない。[設計判断](../../../../../docs/decisions/OC_MANAGEMENT_V1.md)、[共通Runtime](../../../docs/RUNTIME.md)を参照。

`LEGACY_OC_SETTINGS_PATH` の旧管理設定は、Runtime::open → oc::import_legacyで初回だけ取り込む。settings・joinMessages・leaveMessagesを変換し、新版設定を上書きしない。期限切れmuteは取り込まず、未知のURLルール・過大な設定は黙って捨てず起動を止める。旧setupSessionは再利用しない。
