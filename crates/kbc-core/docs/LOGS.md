# 受信ログの受付と同期

2026-10-03。Protocol v6。logs_enabledを指定したCoreが受付を記録する。GitHub設定とOC_LOGS_ENABLED=1が必要。旧ログ変換が完了してから有効化する。

logs::initializeがlog_pendingとlog_membersを同じSQLiteへ作る。logs::ingestは重複排除・baseline判定後、Command処分前に呼ぶ。message rowは本文・contentType・senderName・metadataを保存する。無所属はunmappedへ残す。名前は以前の観測と比較し、違う時だけnamesへ保存する。古い観測で最新名を巻き戻さない。OCとトークのmember-eventを分ける。NAMEだけの更新は参加通知・処分へ渡さない。

check_capacityはBatch末尾でpending件数・byteを検査する。8,192行・8MiB、row64KiB。log_membersは最新8,192人。容量不足はtransaction全体をrollbackし、checkpointを進めない。64MiBのCore DB上限も共有する。

pending_logsは最大8,192行・8MiBのplain DTOを返す。acknowledge_logsは128行までのsequenceを確定削除する。Adapterは32streamまでまとめ、遠隔ファイルとmanifestの保存成功後だけackする。Nativeは型変換とCore呼出のみ。healthへ本文や個別IDを出さず、pendingLogs / pendingLogBytesを観測する。

LogSync.flush / append / runは最新manifest・追記先を取得してmergeする。readVersionedRemoteは同じblob SHAの内容を読む。writeVersionedRemoteはSHAを条件に保存し、競合を再mergeへ返す。書込後のmanifest失敗でもpendingは残り、次回は既存行をhash照合する。全種別を共通の有限Workerで扱う。

normalizeEventsは複数kickeeの通知を個別Eventへ展開する。Receiver.acceptは100件・256KiBのBatchへ分割し、最後まで古いcheckpointを維持する。途中失敗は受付済みIDを重複排除して続け、残りを飛ばさない。profile更新はNAMEへ正規化する。PUSHで届かない過去の名前変更時刻は推定しない。

[階層・配列・同期契約](../../../docs/decisions/OC_LOG_STORAGE_V2.md)、[退避の契約](../../../docs/operations/GITHUB_RECOVERY.md)。
