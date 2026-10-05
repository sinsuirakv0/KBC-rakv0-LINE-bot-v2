# GitHubへの状態退避と復元

2026-10-03。利用者指定により永続Volumeを追加せず、既存の非公開データリポジトリを利用する。オフライン復元試験と本環境でのコンテナ交換・Core復元・変更時の暗号化退避を確認した。[配備と観測値](MINIMAL_BOT.md)。

## 起動と関数の関係

Adapterの `GitHubPersistence.restore` は保存先の非公開設定を確認し、認証とCoreを復元する。`restoreSettings` は旧permissionsとOC設定を機密ファイルへ配置する。`AuthStorage.load`、既存tokenでのlogin、`Runtime::open`の順に進む。旧OC設定はCoreのtransactionで初回だけ取り込み、新版の変更を上書きしない。旧BOT権限はSQUAREのadmin/modとOCの個別・全体停止をCoreへ初回だけ取り込む。

既存の `PUSH_SUBSCRIPTIONS_GITHUB_REPO / TOKEN / BRANCH`、`LINE_STORAGE_BACKUP_KEY` と `LINE_STORAGE_GITHUB_PATH` を利用する。旧 `line-auth/storage.enc.json` は変更しない。新版は `line-auth/v2-reserved-storage.enc.json` と `v2/runtime/core.sqlite.gz.enc.json` へ保存する。旧版と同じSHA256鍵導出・AES-256-GCM形式。認証値、復号済みDB、MIDを公開Botリポジトリへ置かない。

`protectAuth` はLINEJS 3.4.2の `getReqseq` が実通信前に `storage.set("reqseq", ...)` を待つ性質を使う。各namespaceの送信番号を10,000件先までGitHubに予約する。再起動では予約済み上限へ進み、次の区間を確保してから通信する。保存失敗はAuthStorageErrorで全体を停止する。token更新も暗号化した同じファイルへ退避する。新しいnamespaceの最初の処理は予約通信分の遅延がある。番号の飛びは実LINEで確認する必要がある。

`Runtime::persistence_revision` はSQLiteの変更数を返す。`snapshot_database` は共通DB lock下で `VACUUM INTO` を実行する。Adapterの `backup` は一時DBをstream圧縮・暗号化し、GitHubへ保存する。`run` は60秒ごとに変更時だけ実行し、起動前と正常終了時も退避する。生DBをJSON/base64へ変換せず、圧縮結果を最大8MiBに制限する。遠隔ファイル16MiB、復元DB68MiB、HTTP15秒、競合再試行3回。失敗はmetricsへ出し、次の周期で再試行する。

## 復旧の限界

コンテナ交換では最後の成功した退避以後の受付・設定・結果が失われ得る。通常の間隔は約60秒だが、GitHub障害中は長くなる。GitHub保存完了がLINE送信と原子的になるわけではなく、全件永続性やexactly-onceは保証しない。送信待ちが遠隔退避後に実行された可能性があるため、遠隔復元で期限済みの投稿・削除・membership変更・通報をunknownへ移す。未来の予定通知と読み取り照会は維持する。結果を実OCで確認してから明示解決する。

Core snapshotは設定・受信checkpoint・Action・OC短期状態・未同期ログを含む。生成メディアの一時成果は含めない。成果が失われた配送は既存の再実行案内へ切り替わる。unknownの照合で成果の有無だけを根拠にしない。長期ログは5分ごとに取得・追記する別の保存経路で、未同期分はCore退避から再開する。[ログ保存](../decisions/OC_LOG_STORAGE_V2.md)。

Core DBはアカウントOwnerMismatchを検査する。非公開repoでない場合・認証復号失敗・必須旧設定取得失敗は起動を止める。自動的に空設定へ切り替えない。GitHub設定がないローカル実験は既存のローカル保存のみを使う。

## 検証

`npm run smoke:persistence` は外部通信を行わず、旧暗号化認証からの移行、予約内の送信番号更新でGitHub書込が増えないこと、Core checkpointの復元、期限済み投稿の照合待ち、予約失敗でローカル番号が進まないことを確認する。実際の機密設定を使ったローカル検証ではOC設定3件・通知設定5トークを取り込み、再起動時の重複なしを確認した。

関連: [Adapter](../../apps/line/docs/ADAPTER.md)、[Runtime](../../crates/kbc-core/docs/RUNTIME.md)、[OC](../../crates/kbc-core/src/oc/docs/OC.md)。

2026-10-06、旧permissionsのSQUARE admin/mod・OCのbotStops・globalBotStopはCore SQLiteへ初回だけ取り込む。以後のBOT権限・停止設定は同じ暗号化Core snapshotが正本。旧settings/permissions.jsonは初期移行元であり、新版の変更は書き戻さない。snapshot復元後は旧ファイルの内容を優先せず、解除済み設定を戻さない。[機能と上限](../../crates/kbc-core/src/oc/docs/BOT.md)。
