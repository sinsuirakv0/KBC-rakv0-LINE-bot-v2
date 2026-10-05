# 管理下OCの操作テスト

2026-10-04。利用者が管理する検証OCで、実行元と対象トークを変えたときのメンション・削除・強制退会・副官設定・管理人移行のAPI結果を確認する。実LINEのadmin試験でILLEGAL_ARGUMENTと変更なしの利用者報告があり、正規権限での成功とエラー原因の切り分けは未確認。通常の管理機能へ転用する前に結果を記録する。

## OC AとOC Bの権限チェックを調べる目的

利用者が明示した構想は、BotがOC Aの管理人、OC Bでは必要な権限を持たない状態で、Aの権限を根拠にBの管理操作が成立するかを、利用者が管理する検証OCで確認すること。旧アカウントの復旧とは別の検証目的として扱う。以前の一般メンバーの管理人化は、利用者によれば乗っ取りのような事例であり、時期・API・手順は未確認。公式の自動移行だったと断定せず、現在のAPIで再現できる根拠にも扱わない。

現実装はAでの入力・BOT管理者権限確認と結果通知を保持するが、対象InspectでBの親OC・Bot・メンバー・revisionを取得し、Rolesの内容をBの所属で統一する。外側のCoreAction.chat_idはAの受付・継続用で、SquareDirectoryのroles分岐はこれをSDK引数へ渡さない。LINEJS 3.4.2のupdateSquareMembers要求はupdatedAttrsとmembersで、各memberのsquareMid / squareMemberMid / revision / roleへBの値を設定する。送信の認証は共通BaseClientのBotアカウント。Aの管理人MID・role・revisionを変更要求へ混ぜたり、Aの権限を示す認可情報を送ったりはしていない。getSquareAuthorityもOCの操作ごとの必要roleを返す設定で、別OCへ渡す認可tokenとして実装されていない。

このため、現在の実試験は「AからBへの操作指示」であって、「Aを権限確認先、Bを変更先にした要求」の検証済みとは扱わない。Aの現在のBotのOC roleを必須前提として記録する仕組みもまだない。BOT adminとLINEのOC ADMINを混同しない。現在の同一所属の照合は維持する。

切り分けはまず正規権限のある検証OCで同じAPI要求が正常に通ることを確認し、その後、権限のない検証OCへの要求と送信内容・変更前後の状態を比較する。ILLEGAL_ARGUMENTだけで権限境界の判定結果や全経路の不可能性を断定しない。LINE側の認可実装はSDKの要求形式からは確定できない。正常系・OC間の認可情報共有・権限回避の成立はいずれも未確認。

## 登録と実行

BOT admin（実行OCまたは実行トークのrank 2）だけが利用する。OC ADMIN / CO_ADMIN、BOT modでは登録も操作もできない。共通Contextで実行者のMID・OC・JOINEDを確認する。

!test allow <sMID> <sMID> ... で許可する検証OCを登録する。引数名はallow。空指定またはlistは一覧、remove <sMID> ...は解除、addも受け付ける。sで始まるOC MIDを空白で複数指定し、一度16件・全体64件まで。重複はまとめ、不正なMID・上限超過では入力全体を反映しない。登録先はoc_test_squaresで、既存Core snapshotのGitHub退避・復元に含める。ソースや環境変数へ実MIDを埋め込まない。

操作は実行OC・対象OCの両方が登録済みであることを要求する。対象トークのmMIDから現在の親OCとBotを取得し、現在のメンバーのOC・JOINED・role・revisionを照合する。トーク所属とBotの役割を古いcacheだけで判定しない。対象Botが一般メンバーの場合も許可OC内では負試験を1回行える。

| 入力 | 操作 |
| --- | --- |
| !test mention <pMID> -- <本文> | 指定メンバーを実行トークでメンション。本文省略は「メンション通知テスト」 |
| !test delete <数字のメッセージID> | 管理者削除。取り消しではない |
| !test kick <pMID> | KICK_OUT。再参加可能な強制退会 |
| !test deputy on <pMID> | 一般メンバーへCO_ADMINを付与 |
| !test deputy off <pMID> | CO_ADMINをMEMBERへ戻す |
| !test admin <副官pMID> | 現管理人をCO_ADMIN、指定副官をADMINへ同じ要求で変更 |

--target-chat <mMID>は対象トークを指定し、省略時は実行トーク。mentionの場合はpMIDの所属OCを確認するための照会先であり、投稿先は常にコマンドを実行したトークとする。deleteなどの管理操作は指定先へ作用する。--applyなしでは未実行の対象確認、付けた場合だけ実通信を登録する。mentionの本文を指定する--より前にこれらの引数を置く。o.testも同じ。管理操作の対象は1件で、許可OCの複数登録とは別の指定である。

--target-chatはmから始まるトークMID専用。sから始まるOC MIDを受け付けない。対象トークの!id talkでmMIDと親OCのsMIDを確認する。allowには両方のOCのsMIDを登録し、mMIDを混ぜない。admin / deputy / kickは入力トークから解決した親OCに作用するため、同じOCのサブトークごとの権限移行ではない。

2026-10-04の実ログで、対象OCだけを登録し、未登録の実行OCからadminを呼ぶと実行OCの判定で停止することを確認した。また、不正な--target-chatのsMIDが同じ判定に隠れていた。現在はBOT権限を確認した後、引数の形式を先に検証する。未登録の実行OC・対象OCは別の文面で実際のsMIDと登録コマンドを返す。Inspect待機中に実行OCの登録を解除した場合も、対象OCの未登録と混同しない。操作の許可範囲やAPI引数は変更しない。

adminの移行元は対象OCのBot。Botが対象OCの管理人でない場合は、--from <現在の管理人pMID>を明示して権限不足の試験も行える。現在ADMINとCO_ADMINである2者のrevisionを取得し、同じ対象OCであることを照合する。副官の「譲渡」はon/offとして役割の付与・解除を試す。kickは一般メンバーだけが対象で、Bot・ADMIN・CO_ADMIN・未知roleを除外する。

管理操作はAPI上で対象トークまたは対象OCを指定する。実行元トークの権限を引き継ぐ引数はない。対象側に権限がない場合の可否はサーバーの判定と実観測で確認し、権限回避が成立すると仮定しない。メンションは照会先OCのpMIDを実行トークのMENTIONへ設定する実験で、描画・API受理・端末通知は別に観測する。管理人移行の一般仕様は[LINE公式](https://help.line.me/line/smartphone?contentId=20005393&lang=ja)で現在の副官への移行・旧管理人の副官化を確認した。

!test replyの--chatは返信元トークを表す。今回の--target-chatと混同しない。replyの既存仕様・権限は変えず、allow・--applyも要求しない。[リプライ表示の実験](TEST_REPLY.md)。

!test sticker <セットID> <スタンプID>は実行トークへのスタンプ送信試験。BOT管理者の共通Context・Mutation・結果履歴を使うが、allow登録・--applyは不要。[仕様・関数・検証範囲](TEST_STICKER.md)。

## SDKと結果

2026-10-04のnpm再確認でも公開版はLINEJS 3.4.2。採用lockを維持する。SquareMemberAttribute.ROLEは6、役割はADMIN=1 / CO_ADMIN=2 / MEMBER=10。updateSquareMembersのupdatedAttrs=[ROLE]とmembersを使用し、squareMid・squareMemberMid・revision・roleだけを送る。SDK配布物の型・Thrift serializerで要求と返値を確認した。返されたmembersで対象・OC・役割を照合できなければ成功と推定しない。

メンションは既存MessageMentionと同じUTF-16位置でMENTION.MENTIONEESを設定し、relatedMessageIdを付けない。ラベルは@表示名。本文込み1,500 UTF-16単位を超える入力を分割送信しない。削除はdestroyMessage(squareChatMid, messageId)、退会は既存updateSquareMember(updatedAttrs=[5], KICK_OUT)を再利用する。

2026-10-04、利用者の訂正でmentionの送信先を修正した。test::planのchatは照会先として保持し、test::inspectedでInspectの所属・JOINED・allowを確認後、OcRequest::Post.chat_idだけを元のJob.eventの実行トークへ設定する。確認表示に送信先、test::mutatedの結果にメンバー照会トークと送信先を表示する。Plan・Protocol・保存形式は変更しない。oc_historyのOCキーは従来どおり照会対象OCで、他の管理操作・権限・期限・unknownの再送抑止は維持する。

同じsmoke:ocで、別OCのpMIDを指定先でInspectしながら実行トークへ投稿すること、サブトークでの実行・Mutation登録後の再起動・指定省略時の同OCメンションを確認した。既存の所属不一致拒否・対象OC登録・delete / kick / rolesの対象指定も通過した。build、型検査、smoke:oc / smoke:commands、Clippyが通過。helpは1,317 UTF-16単位。実LINEの別OCメンション表示・通知は未確認。

Protocol v8のInspect / Roles / Post / Deleteを既存OcApiへ追加する。独立したQueue・Worker・HTTP・常駐巡回は作らない。Inspectは既存1照会Worker、変更は既存2配送Worker・ApiSchedulerの共通2枠を使う。1要求のInspectはトーク・Bot・最大2メンバーを取得する。参加者一覧や未知OCの常時照会は増やさない。

受信から照会完了まで60秒、変更Actionの実通信開始まで登録後30秒を受付期限とする。通信前失敗だけ既存再待機、実fetch直前にsendingを永続化する。通信後の例外・応答照合不成立はunknownとして保存し、自動再実行しない。権限拒否を含め安全なAPI codeを実行トークへ表示し、生のSDK例外を転送しない。再実行前に対象の状態を確認する。

API成功・失敗・結果不明を実行トークへ返し、対象OCをキーにoc_historyへ保存する。通信結果不明時のcontinuationは既存Outboxだけに保持し、同じJobを別tableへ複製しない。運用者によるresolveActionは履歴を更新し、同じ結果通知を再投稿しない。許可解除は次の対象照合から反映し、照合後に既に登録された操作を取り消す機能ではない。

## 実LINEのadmin試験とアカウント復旧

2026-10-04の利用者報告: !test adminに対象トーク・--from・--applyを指定すると、ILLEGAL_ARGUMENT / 結果不明となり、管理人権限の取得は観測されなかった。実MIDは資料へ転記しない。送信前の対象照合は通ったが、管理人権限を持つBotで同じ要求の成功はまだ確認できていないため、権限不足と要求形式・状態の不整合をこの1件だけで区別しない。

--fromは変更対象の現在の管理人を指定する。認証アカウントや通信の実行者を変更しない。adminは指定副官を管理人へ変更する試験で、自分の副官資格をBotへ移す操作ではない。通信後の例外を共通配送がunknownへ分類しているため、「結果不明」という表示を成功や部分的な権限取得の証拠にしない。自動再送しない。

利用者の旧管理人アカウントはログイン・引き継ぎ不能で、公式への問い合わせでも解決しなかったとの報告。[LINE公式の管理者仕様](https://help.line.me/line/smartphone?contentId=20005393&lang=ja)では共同管理者の追加・削除と管理者から副官への管理者移行が案内され、アカウント削除時には参加期間が最も長い副官、次に一般メンバーへ自動移行する。単なるログイン不能を削除済みとは扱わない。

[LINEによる不在管理者の移行](https://openchat-jp.line.me/other/notice_admin_transfer_by_LINE_aD84jLf2)は管理機能の長期喪失等で行われる場合があるが、対象・時期・移行先の希望を受け付けず、定常的な実施も約束していない。過去に一般メンバーが管理人となった観測を、APIの権限確認回避が成立した証拠にしない。[本人のアカウント引き継ぎ条件](https://help.line.me/line?contentId=20000098&lang=ja)と有効な既存ログインの有無、正規権限でのAPI正常系を次の切り分け対象にする。いずれも未確認であり、旧アカウントへの復旧やBotへの役割変更を成功済みとは記録しない。

## 関数と検証

| 関数 | 接続と働き |
| --- | --- |
| test::initialize / allow / permitted | 小さな検証OC table、原子的な複数登録・解除、所属OCの照合 |
| test::parse / plan / execute | 共通ContextのBOT権限、理由付き引数解析、実行OC登録確認、Inspectへの継続Job登録 |
| test::inspected | 現在の対象・役割・期限・許可を確認し、previewまたは1操作を登録 |
| test::mutated | 既存ActionResultから結果通知・対象OCの履歴、手動解決時の重複通知抑止 |
| oc::ingest / complete / request | 共通受付・transaction・Outbox・continuationを共有 |
| SquareDirectory.execute / mentionMetadata | Inspectの現在照会、SDK操作、返値の所属・役割照合。通常投稿とテスト投稿でMENTION変換を共有 |
| deliverAction / ApiScheduler | API名ごとの実送信境界、30秒期限、共通枠とunknown契約 |

smoke:ocの同じSDK mockで、BOT権限、複数許可登録・不正入力の非反映、previewで変更しないこと、別OC・一般roleのBotの要求先、MENTION metadata、管理者削除、KICK_OUT、役割付与・解除、2者の管理人移行、メンバー所属不一致、許可解除、通信後拒否をunknownにして再送しないこと、手動解決の履歴更新と再起動後の許可保持を確認した。実LINEで権限不足の操作が通ることや通知が鳴ることの証明にはしない。

誤入力の修正は同じSmokeで、実行OC未登録でも--target-chatのsMIDを先に拒否すること、対象だけ登録した場合の実行OCの案内、対象解除後の親OCとトークの案内を確認した。誤入力から役割変更を送らない。npm run build、smoke:oc、smoke:commands、Clippyの同じ設定が通過。help本文は974 UTF-16単位で、1,500単位の上限内。

Windows / Node 24.15.0 / Rust GNU LLVMでnpm run build、smoke / smoke:commands / smoke:oc / smoke:persistence / smoke:logsとClippy（workspace・all-targets・release・locked・警告をエラー扱い）が通過。途中のオフライン検証でinspectを照会Action選択のSQLへ登録し忘れる待機を検出し、修正後に同じ経路の完了を確認した。BOM・LF・資料リンク・git diff --checkも確認した。

2026-10-05、実機試験後の利用者指定によりmention-label（自由表示名・同範囲の複数人メンション）を廃止した。既存の!test mentionは維持する。Protocol番号は混在を避けるためv15を維持し、複数人用DTOは削除した。
