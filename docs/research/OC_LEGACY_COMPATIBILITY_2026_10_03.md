# 旧OC管理の互換差と今回の修正

旧版 [c6796e0](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/tree/c6796e05ad76397acbd1e17f0635924e08f1d54e)、新版86c1ca8を比較。利用者指定のChrome GPTへ公開ソースの調査を依頼し、思考を中断せず22分38秒後の回答を取得した。[調査の会話](https://chatgpt.com/c/6abf8f53-81ec-83e8-85a0-9e83ae1e9aff)。機密repo・認証・実OCログはGPTへ渡していない。以下は提案をそのまま採用せず、ローカル旧コードと照合した内容。

| 差 | 旧コードの根拠 | 対応 |
| --- | --- | --- |
| kicktestがkickへaliasされていた | [oc.ts 2394](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/commands/oc.ts#L2394-L2397)は案内だけ | 処分しない案内へ修正 |
| KICK_OUT / BANNEDを通常leave本文へ流していた | [squareMembership.ts](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/moderation/squareMembership.ts#L9-L27)はJOINED / LEFTだけを通常参加退出として分類 | 強制退会・再参加禁止は別種のログを維持し、通常leave本文へ変換しない。別途LEFTが届く場合の実配信は観測対象 |
| 再参加者の短時間退出にも審議を作っていた | [leftSoonMode](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/moderation/ocModeration.ts#L1560-L1566)は再参加者をlogへ分類 | 自動処分・審議を作らず副官ログだけ |
| 手動kickの結果・履歴がMIDと成否中心 | [executeKick](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/commands/oc.ts#L2270-L2319)、[履歴](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/commands/oc.ts#L2018-L2045) | 名前・MID・理由、副官ログの実行者・実行トークを追加。kick historyを処分だけへ絞る |
| muteの期限・mention・警告削除が省略 | [sendMuteWarning](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/moderation/ocModeration.ts#L943-L978) | 期限・残り時間・mention、送信ID確定後15秒の管理者削除。警告抑制60秒は維持 |
| mention+短縮IDの順序、leftmessage等のalias差 | [ocJoinMessage.ts](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/moderation/ocJoinMessage.ts#L130-L141) | mention→短縮ID→本文。leftmessage、審議の処分/キック/対応不要/不要/再参加禁止解除を追加 |

手動kickは旧版も直接BANNED。[ocSquareBan.ts](https://github.com/sinsuirakv0/KBC-rakv0-line-bot/blob/c6796e05ad76397acbd1e17f0635924e08f1d54e/src/moderation/ocSquareBan.ts#L32-L65)の実体と表示上の「強制退会＋再参加禁止」を区別する。危険語のKICK_OUTと追加BANNED審議は維持する。失敗・結果不明を成功扱いせず、旧版の失敗でも成功と書く文面や審議の実行者名bot固定は引き継がない。本人限定のsetup session、有限上限、新旧権限区分も維持する。

通常応答は入力へのrelatedMessageIdを付けない。削除済みの原因投稿へreplyしてNOT_FOUNDになる経路を作らない。一斉参加の監視など原因投稿を残した副官通知は、同じOCのサブトークをまたぐreplyに結び付ける。別OCへのreplyは未確認。旧idの移植と新しいreply情報取得は [ID実装](../../crates/kbc-core/src/oc/docs/ID.md) を参照。

検証は既存smoke:ocへ少数の実際の回帰条件を追加する。kicktestでmembership APIを呼ばないこと、通常ID応答が非replyであること、サブトーク参照を確認済み。実処分の網羅的な再実行や、GPT回答だけを根拠に旧挙動の復元を行わない。
