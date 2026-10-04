# 管理下OCの操作テスト

2026-10-04。利用者が管理する検証OCで、実行元と対象トークを変えたときのメンション・削除・強制退会・副官設定・管理人移行のAPI結果を確認する。実LINEでの権限境界の確認は未実施。通常の管理機能へ転用する前に結果を記録する。

## 登録と実行

BOT admin（実行OCまたは実行トークのrank 2）だけが利用する。OC ADMIN / CO_ADMIN、BOT modでは登録も操作もできない。共通Contextで実行者のMID・OC・JOINEDを確認する。

!test allow <sMID> <sMID> ... で許可する検証OCを登録する。引数名はallow。空指定またはlistは一覧、remove <sMID> ...は解除、addも受け付ける。sで始まるOC MIDを空白で複数指定し、一度16件・全体64件まで。重複はまとめ、不正なMID・上限超過では入力全体を反映しない。登録先はoc_test_squaresで、既存Core snapshotのGitHub退避・復元に含める。ソースや環境変数へ実MIDを埋め込まない。

操作は実行OC・対象OCの両方が登録済みであることを要求する。対象トークのmMIDから現在の親OCとBotを取得し、現在のメンバーのOC・JOINED・role・revisionを照合する。トーク所属とBotの役割を古いcacheだけで判定しない。対象Botが一般メンバーの場合も許可OC内では負試験を1回行える。

| 入力 | 操作 |
| --- | --- |
| !test mention <pMID> -- <本文> | 指定メンバーをメンション。本文省略は「メンション通知テスト」 |
| !test delete <数字のメッセージID> | 管理者削除。取り消しではない |
| !test kick <pMID> | KICK_OUT。再参加可能な強制退会 |
| !test deputy on <pMID> | 一般メンバーへCO_ADMINを付与 |
| !test deputy off <pMID> | CO_ADMINをMEMBERへ戻す |
| !test admin <副官pMID> | 現管理人をCO_ADMIN、指定副官をADMINへ同じ要求で変更 |

全操作に--target-chat <mMID>を付けると対象トークを変える。省略時は実行トーク。--applyなしでは未実行の対象確認、付けた場合だけ実通信を登録する。mentionの本文を指定する--より前にこれらの引数を置く。o.testも同じ。管理操作の対象は1件で、許可OCの複数登録とは別の指定である。

adminの移行元は対象OCのBot。Botが対象OCの管理人でない場合は、--from <現在の管理人pMID>を明示して権限不足の試験も行える。現在ADMINとCO_ADMINである2者のrevisionを取得し、同じ対象OCであることを照合する。副官の「譲渡」はon/offとして役割の付与・解除を試す。kickは一般メンバーだけが対象で、Bot・ADMIN・CO_ADMIN・未知roleを除外する。

各操作はAPI上で対象トークまたは対象OCを指定する。実行元トークの権限を引き継ぐ引数はない。対象側に権限がない場合の可否はサーバーの判定と実観測で確認し、権限回避が成立すると仮定しない。メンションの描画・metadataの付与と端末通知は別に観測する。管理人移行の一般仕様は[LINE公式](https://help.line.me/line/smartphone?contentId=20005393&lang=ja)で現在の副官への移行・旧管理人の副官化を確認した。

!test replyの--chatは返信元トークを表す。今回の--target-chatと混同しない。replyの既存仕様・権限は変えず、allow・--applyも要求しない。[リプライ表示の実験](TEST_REPLY.md)。

## SDKと結果

2026-10-04のnpm再確認でも公開版はLINEJS 3.4.2。採用lockを維持する。SquareMemberAttribute.ROLEは6、役割はADMIN=1 / CO_ADMIN=2 / MEMBER=10。updateSquareMembersのupdatedAttrs=[ROLE]とmembersを使用し、squareMid・squareMemberMid・revision・roleだけを送る。SDK配布物の型・Thrift serializerで要求と返値を確認した。返されたmembersで対象・OC・役割を照合できなければ成功と推定しない。

メンションは既存MessageMentionと同じUTF-16位置でMENTION.MENTIONEESを設定し、relatedMessageIdを付けない。ラベルは@表示名。本文込み1,500 UTF-16単位を超える入力を分割送信しない。削除はdestroyMessage(squareChatMid, messageId)、退会は既存updateSquareMember(updatedAttrs=[5], KICK_OUT)を再利用する。

Protocol v8のInspect / Roles / Post / Deleteを既存OcApiへ追加する。独立したQueue・Worker・HTTP・常駐巡回は作らない。Inspectは既存1照会Worker、変更は既存2配送Worker・ApiSchedulerの共通2枠を使う。1要求のInspectはトーク・Bot・最大2メンバーを取得する。参加者一覧や未知OCの常時照会は増やさない。

受信から照会完了まで60秒、変更Actionの実通信開始まで登録後30秒を受付期限とする。通信前失敗だけ既存再待機、実fetch直前にsendingを永続化する。通信後の例外・応答照合不成立はunknownとして保存し、自動再実行しない。権限拒否を含め安全なAPI codeを実行トークへ表示し、生のSDK例外を転送しない。再実行前に対象の状態を確認する。

API成功・失敗・結果不明を実行トークへ返し、対象OCをキーにoc_historyへ保存する。通信結果不明時のcontinuationは既存Outboxだけに保持し、同じJobを別tableへ複製しない。運用者によるresolveActionは履歴を更新し、同じ結果通知を再投稿しない。許可解除は次の対象照合から反映し、照合後に既に登録された操作を取り消す機能ではない。

## 関数と検証

| 関数 | 接続と働き |
| --- | --- |
| test::initialize / allow / permitted | 小さな検証OC table、原子的な複数登録・解除、所属OCの照合 |
| test::parse / plan / execute | 引数解析、共通ContextのBOT権限、Inspectへの継続Job登録 |
| test::inspected | 現在の対象・役割・期限・許可を確認し、previewまたは1操作を登録 |
| test::mutated | 既存ActionResultから結果通知・対象OCの履歴、手動解決時の重複通知抑止 |
| oc::ingest / complete / request | 共通受付・transaction・Outbox・continuationを共有 |
| SquareDirectory.execute / mentionMetadata | Inspectの現在照会、SDK操作、返値の所属・役割照合。通常投稿とテスト投稿でMENTION変換を共有 |
| deliverAction / ApiScheduler | API名ごとの実送信境界、30秒期限、共通枠とunknown契約 |

smoke:ocの同じSDK mockで、BOT権限、複数許可登録・不正入力の非反映、previewで変更しないこと、別OC・一般roleのBotの要求先、MENTION metadata、管理者削除、KICK_OUT、役割付与・解除、2者の管理人移行、メンバー所属不一致、許可解除、通信後拒否をunknownにして再送しないこと、手動解決の履歴更新と再起動後の許可保持を確認した。実LINEで権限不足の操作が通ることや通知が鳴ることの証明にはしない。

Windows / Node 24.15.0 / Rust GNU LLVMでnpm run build、smoke / smoke:commands / smoke:oc / smoke:persistence / smoke:logsとClippy（workspace・all-targets・release・locked・警告をエラー扱い）が通過。途中のオフライン検証でinspectを照会Action選択のSQLへ登録し忘れる待機を検出し、修正後に同じ経路の完了を確認した。BOM・LF・資料リンク・git diff --checkも確認した。
