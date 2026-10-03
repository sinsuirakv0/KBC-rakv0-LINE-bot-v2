# OCログの軽量保存と容量による統合

更新日: 2026-10-03。最新の利用者指定を採用。旧ログを廃棄する以前の方針を置き換え、可逆変換して引き継ぐ。Workflowで本データの変換・照合・旧フォルダ整理を完了。[反映結果](../../scripts/logs/docs/MIGRATION.md)。本環境の同期結果は運用資料へ記録する。

## 階層と形式

logs/v2/s<OC MID>/を親にする。OC全体のnames/とmember-events/を置き、m<トーク MID>/の下にmessages/、member-events/、旧profiles/を置く。各末端はmanifest.jsonと000001.jsonl.gz等。旧個人・グループはtalk/、所属不明はunmapped/へ残す。実MIDは機密データrepoだけに保存する。

JSONLの先頭はversion・kind・contextのheader。contextはフォルダから導けない例外だけを保存する。各行は配列で、末尾の不要なnullを省く。keyとOC・トークMIDを発言ごとに繰り返さない。gzipで本文や送信者の反復も圧縮する。

| 種別 | 行の並び |
| --- | --- |
| messages | createdAt, messageId, senderMid, content, contentType, senderName, metadata, extra |
| member-events | at, type, memberMid, name, extra, 通知元chat（OC全体の場合） |
| names | firstSeenAtまたはat, memberMid, before, after, lastSeenAt, count, extra |
| profiles | 旧profile object。stream contextを共用 |

metadataは [object,mask]。1=to、2=toType、4=squareChatMid、8=squareMid、16=eventCreatedTimeのcontext一致項目を省略した印。フォルダcontext・例外header・行の時刻で復元する。違う値は省略しない。extraは未知のrecord属性とcontextとの差を保持する。旧名前観測はbefore=null、範囲と回数を保持し、正確な改名時刻へ読み替えない。新版はbefore / afterと観測時刻を保存する。

## 保存周期と切替

5分ごとの保存ではGitHubからmanifestと現在の追記先を取得し、新着を追加して同じファイルを更新する。非圧縮4MiBを目安に、次の行が入らない場合だけ次の番号へ切り替える。時間・日付・月だけでは新ファイルを作らない。低頻度OCは長期間同じファイル。旧ログも同じ容量単位でまとめる。

現在のファイル更新後、manifest更新まで終わってからCoreでackする。途中失敗・再起動では新しいSHAを取得して再mergeし、同じ行を増やさない。SHA競合は最大3回読み直す。切替途中の次番号ファイルも取得する。復元で戻った未ack行は最新3ファイルと照合する。確定ファイルのchecksumを照合するが、更新途中のactive manifestのhashは確定保証として使わない。

1周期は最大32stream。全履歴を起動時に読み込まず、現在ファイルと直近の確認対象だけをstreamごとに取得する。全文検索の索引と新log commandは未実装。将来はOC・トーク・必要なファイルへ絞って検索する。

## 受付と復旧

Rust Coreは受信transactionでpendingを保存する。上限8,192行・8MiB、row64KiB。超過Batchはcheckpointを進めずrollbackする。満杯を無言で削除しない。Protocol v7のplain DTOを使い、SDK objectを渡さない。ログの圧縮・GitHub入出力はAdapterの共通Worker。

pendingはCoreの毎分暗号化snapshotにも含まれる。長期ログ同期が5分でも、Core snapshotから未同期分を再開する。最後の成功退避以後はコンテナ消失で失われ得る。単一BOTで書き込み、移行Workflowが運用中の追記先を置き換えない。[保存・復旧の限界](../operations/GITHUB_RECOVERY.md)。

旧ログはActionsで変換・照合後に整理する。元状態のbackup branchを作って復旧可能にする。認証・設定・Coreを整理対象へ混ぜない。破損原ファイルと旧集約索引も新形式側へ残す。[変換関数・Workflow・実測](../../scripts/logs/docs/MIGRATION.md)。

参加退出の新版extraには受信元source・元eventType・receivedAtMsを保存する。常時取得の遅れを比較するための観測値で、元のイベント時刻を新着時刻に置き換えない。行の配列schemaと旧ログの可逆復元は変更しない。
