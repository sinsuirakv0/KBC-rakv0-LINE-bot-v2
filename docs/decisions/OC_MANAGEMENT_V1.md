# OC管理の移植と永続Action

2026-10-03。設計採用、実装・オフライン検証済み。旧LINEのoc.tsと実際のmoderation処理を参照する。probe/identityは利用者の指定で後回し。

OC全体の設定・BOT権限はsquareMid、入退室の送信先はsquareChatMidを使う。初期値は自動処理OFF。旧ログを廃棄する方針と管理設定・認証の継承は分け、旧ストレージを削除しない。設定・短期状態・対話・審議は既存SQLiteへ保存する。GitHubへの退避・復元は既存計画の後続作業。

OC APIは型付きOcRequest/Resultを既存Outboxへ接続する。解析・権限判断・設定・次の操作はRust、SDK呼出とDTO変換はAdapter。照会は再実行できるが、削除・membership更新・通報の通信開始後は結果不明を自動再送しない。取得結果と後続Actionを同じtransactionで保存する。新しいCommand専用Queue/HTTP基盤は作らない。

通常会話の全件でmember照会を行わず、安価な候補判定に一致した場合と管理Commandで現在の権限を照会する。chat→OC/botの対応はSDK情報の有限cacheで正規化する。cacheは権限を許可するために使わない。対象のOC不一致・権限不明は処分しない。

権限は旧版を維持する。kickはBOT mod以上、muteはBOT mod以上またはOC ADMIN、設定変更はBOT mod以上またはOC ADMIN/CO_ADMIN、setup・副官部屋・入退室設定・審議はBOT adminまたはOC ADMIN/CO_ADMIN。BOT権限は明示した機密permissionsファイルから読む。旧コードの個人MIDを公開ソースへコピーしない。

旧資料と異なり、現行ocSquareBan.tsの手動kickは直接BANNED。通常の危険語自動処分はKICK_OUTだけ、確認リプライでBANNED。即抜けはOC全体の退会を確認でき、記録した初参加から5分以内の場合だけBANNED。サブトーク退室からOC退会を推測しない。PUSHから得られない参加を常時巡回で補う処理は今回持ち込まず、検知範囲を実運用で観測する。

新しい呼び方はjoin/leave、media on/off、main、historyなどに整理し、従来のjoinmes/leavemes/del/hisもaliasとして受け付ける。setupと副官部屋の対象選択、審議は送信済みの本人・トーク・最新promptへのリプライを使う。通常の数字や別人の返信は設定変更にしない。

読み取りは既存Outboxのquerying状態と1本の照会loopで処理し、通常の2配送枠を使わない。全APIの2並列・開始間隔は共有する。通常操作で不要なBot role取得とbaselineより古い履歴のOC取得は省く。これはAPI回数の削減であり、多OC性能の実測完了ではない。

登録済みo.コマンドの先頭語をURL検出から除外し、引数のURLは検査する。巨大なURLは削除候補にするが審議用に複製しない。ノートURL削除は権限免除・イベント取得の調査を後続へ回す。

仕様・関数・上限・検証は [OC管理実装](../../crates/kbc-core/src/oc/docs/OC.md)。実LINEのAPI互換性、参加PUSHの網羅性、配備・保存復元は未確認。
