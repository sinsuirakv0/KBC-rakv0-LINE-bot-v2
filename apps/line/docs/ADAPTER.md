# 最小LINE Adapter

更新日: 2026-10-03。v6の実PUSH・補完・保存復元は本環境で確認済み。以下のv7追加はbuild・通信なしの回帰検証を実施し、本番観測を運用資料へ記録する。全件受信・長期安定性は未確定。

## 関数と処理経路

2026-10-04、NOTIFIED_LEAVE_SQUARE_CHATのsquareMember.displayNameを正規化DTOへ保持する修正を追加した。採用SDKのSquareEventNotifiedLeaveSquareChatのThrift項目でsquareMemberを確認。名前がない場合の観測名参照と通知対象だけの追加照会は[Core](../../../crates/kbc-core/src/oc/docs/OC.md)で行い、Adapterの受信中に名前APIを追加しない。

`main → AuthStorage.load → SDK login → createCore → Receiver.run + 2本の配送loop + 2本の照会loop`。

| 関数・状態 | 働き・相互関係 |
| --- | --- |
| `AuthStorage` | SDKの認証・reqseqを直列更新し、tmp書込・fsync・renameで保存。成功後だけメモリを更新。保存エラー・64件の待機上限到達をglobal abortへ伝え、以後の読書きを拒否。`flush`は全保存を待つ |
| `installApiScheduler` | 3.4.2の`requestCore`を包み、通常・LEGY・SDK token更新を共通枠へ接続。RPC本文の処理完了まで枠を持つ |
| `ApiScheduler.run / pace / pump` | 最大2実行・32待機のFIFOと開始間隔250ms。enqueue・完了・cooldown終了で自律的に起きる。初期値はLINEの許容制限値ではない |
| `Receiver.session` | OCのservice 3だけをHTTP/2で購読。初期応答をSDK Thriftで読み、SDK既定Stream・先行cursor更新を使わない |
| `accept` | SDK Eventをplain DTOへ変換。sender・REPLYの返信先もDTOへ正規化し、非同期Nativeのcommit後だけSDKの再接続用syncを進める |
| `completePending / drainChat` | 必要と示されたchatだけ最大2系統、1回4ページずつ補完。同じchatは直列。失敗をトーク別の期限付き再試行へ変え、位置を残す |
| `runPolling / fetchChat` | 投稿から独立したトークイベント取得。PUSH補完と同じcursor・最大2枠を共有 |
| `Runtime::priority_chats / NativeCore.priorityChats` | Rustの通知・監視設定から短周期の対象を返す。AdapterはCommand固有の判断を持たない |
| `deliverAction` / 配送loop | `nextAction`で`claimed`を取り出す。API枠・reqseq保存・通信準備後、`client.fetch`直前の`beforeFetch`で`markSending`する。通信前失敗は再待機、通信後の例外は`unknown`。次の受信入力を必要としない |

PUSHはdirtyフラグに集約し、通知ごとにTaskやQueueを増やさない。account cursorを取得・保存するloopは一つ。取得中・継続ページ中に来たdirtyは全ページ終了後の再取得まで残す。補完待ちchatと`chatRetries`（試行数・再試行時刻）を全体checkpointへ保存し、補完終了後だけ消す。最大256トークを持ち、上限では黙って捨てず停止。最初のchat取得でも保存した初回originより前の履歴Commandは実行しない。

局所補完の失敗は2秒から最大15分のbackoffにし、account PUSHを開始・継続する。未完了の継続ページは待ち列の末尾へ戻し、新着へ譲る。PUSH待機中は最も早い補完期限でも起き、chatだけを再取得する。補完のために空の`fetchMyEvents`を追加しない。`pendingChats` / `chatFailures`で部分障害を観測し、必要ならローカルcheckpointから対象を照合する。

PUSH接続・ACKはRPC枠を占有しない。初期sign-onも間隔制御を通し、回数は`receiver.signOns`へ記録。短時間RPC回数は`api.methods`で観測する。account取得はPUSH通知、継続ページ、subscriptionのttl期限（80%地点）で起きる。ttl不明時は30分を仮値とする。

HTTP 429の数値Retry-After、`EXCESSIVE_ACCESS`等の識別できる制限応答は全体cooldownへ反映。既定60秒、最大600秒。SDK token更新・再要求は親RPCの枠内で実行し、枠の循環待ちを避ける。RPC名のAsyncLocalStorageにより、送信に伴うrefresh通信を送信開始として扱わない。通信前の再待機以外にアプリ独自の送信再試行はしない。数値だけの未知codeや恒久的アカウント制限の正規化は今後の観測対象。

## 接続・容量・停止

HTTP/2の初期接続とsign-on待ちは各15秒。PUSH入力の組立Bufferは1MiB、SDKへ書込む未消費のBufferは64KiBを上限とし、ACK失敗の未監視rejectをcatchする。無フレーム90秒で接続を閉じる。定期チェックは30秒、keepaliveの`noop`も共通RPC枠へ通す。

各取得は100件、accountの連続100ページで一度復旧へ戻り、保存済みcontinuationから再開する。補完の指示は1ページ最大100chat、持ち越しと合わせ最大256chat。通常接続復旧は1〜60秒のbackoffと小さなjitter。旧接続と取得が終わるまで次の接続を開始しない。Core保存失敗・checkpoint破損・容量超過は局所復旧へ逃がさずプロセス停止する。

終了はglobal signalで通信を取消し、Coreの待機を起こす。`claimed`は次回起動で再待機、`sending`は`unknown`になり自動再投稿されない。Receiverはreadとkeepalive noopをjoinし、mainは受信・配送終了後にStorage全体の`flush`を待つ。flush失敗でもhealth server等の後始末は実施する。停止の強制上限20秒。SDK内部のLEGY通信はSDK自身の15秒timeoutで取消す。DBと外部通信は原子的でないため、送信開始記録直後の終了は結果不明になり得る。

## 今回の対応範囲と観測

コマンドはtxtの応答・help、ut/tut/stの検索・番号リプライ・origin画像、確認用test-notify。[Command実装](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)を参照。ping本文は旧LINEのpong!、登録と引数解析はDiscordのcatalog方式。個人・グループ、LINE thread固有メッセージ、旧Botの停止設定・一般通知機能は未対応。通常のOC管理は追加済み。

未知イベントは種別と件数を最大64種で観測し、媒体・対象の参加退出もCoreへ正規化する。ログには本文・トークID・message ID・認証値・SDKの生の例外を出さない。受信後の詳細はローカルSQLiteに残る。ログはstdoutの集計と配送結果のみ。毎分CPU（1core比）、RSS・heap、API・受信・配送・容量を出す。`/health`は受信ready時200、それ以外503。PUSH heartbeatだけで全メッセージの受信成功を保証しない。

LINEJSの最新公開版は2026-10-03のnpm再確認でも3.4.2。npm配布物revision 11をlock。そこでreqseq初回並列の直列化を確認した。SDKが依存するThrift 0.20はnpm auditでhighが出たため、0.25.0へoverride。実SDKのCompact Protocol初期応答を模擬検証し、audit 0件を確認。通信先の実互換性は少数OCの実験で確認する。

## Commandの追加境界

Protocol v21。通常返信の実送信IDをCoreへ渡し、候補promptへ結び付ける。管理者削除はsquare.destroyMessageへ渡し、新しい返信成功後に実行する。OCのメディアはoid省略のOBS reqseq upload自身が投稿し、空のIMAGE/VIDEOを先に送らない。画像・動画・GIF・ファイルの素材準備はRust共通Worker、BlobとLINEJS入出力はAdapterが扱う。uploadMediaも共通API枠・実fetch直前のsending記録を通し、HTTP statusを共通transportで検査する。動画durationはCoreの実Frame数から渡す。メディア自体はrelatedMessageId付き返信にならない。通信後の不明結果はunknownで自動再投稿しない。[Media Worker](../../../crates/kbc-core/docs/MEDIA.md) と [実素材実験](../../../experiments/commands/docs/MEDIA_VERIFICATION.md) を参照。

通知47のNICE / LOVEは既存ProtocolのReactionNotifiedへ正規化するが、ページ操作には使わない。利用者指定でリアクション方式を廃止し、一覧へのテキストリプライへ戻した。SquareDirectoryは旧snapshotのReactions要求を外部通信なしで完了し、getMessageReactionsを呼ばない。[旧案の調査・スタンプの仕様と未確認点](REACTIONS_AND_STICKERS.md)。

messageMetadataは保存済みMessageEmojiをREPLACE.sticon.resourcesへ変換し、既存MENTIONと合わせてsendMessageへ渡す互換処理を維持する。現在の検索案内はテキストだけを使う。受信したSTKPKGID / STKID / REPLACEは既存metadataJsonに含まれ、Coreの!id sticker / emojiが必要項目だけ有限保存して参照する。履歴照会・巡回APIは増やさない。

OcRequest::StickerはcontentType=STICKERとSTKPKGID / STKID / STKVER / STKTXT / 任意STKOPTをsendMessageへ渡す。送信先はActionの実行トークで、返値のmessage ID欠落は例外としてunknownへ残す。既存配送・30秒受付期限・sendMessageの実fetch境界を共有し、自動再送しない。[!test stickerの仕様・関数](../../../crates/kbc-core/src/oc/docs/TEST_STICKER.md)。

## OCイベント・管理API

normalizeEventは本文なしの画像・動画、OC全体のmember状態、トーク内の参加退出を正規化する。時刻・scope・状態を分け、関連member作成時刻がない場合は初参加と推定しない。SquareDirectoryはchat→OC/botを512件・10分でcacheし、32件まで同じ照会をまとめる。roleはcacheから許可せず現在のgetSquareMemberで確認する。SDK返値のMIDとrevisionを検査する。baseline以前の履歴ではOC追加取得をしない。

Profile要求は実行トークのOC / Bot MIDと照合して、updateSquareMemberのDISPLAY_NAMEだけを更新する。引数なしとrevisionを検査する。Bot独自の文字数・改行・制御文字の制限は設けず、未加工の名前を渡す。Member照会では表示用DTOとは別のrawMemberNameを返す。Profile更新応答は完全なDTOとして扱わず、その後にgetSquareMemberを1回行い、SDKの未加工displayName・所属・参加状態と比較して成功を確認する。[Bot名変更の入力・権限・関数](../../../crates/kbc-core/src/oc/docs/BOT.md)。

context / member / chats / members / joinedChatsはnextQueryActionから2本の照会loopで取得する。通常配送は2本のままで、全RPCは既存ApiSchedulerの同じ上限を共有する。membershipはupdateSquareMember(updatedAttrs=[5], revision付き)、通報はreportSquareMessage(SCAM)。更新・通報も実fetch直前にsendingを保存し、通信後失敗を自動再試行しない。読み取りはfailedで確定でき、再起動は再取得する。

入退室のメンションはRustがUTF-16位置を作り、AdapterはMENTIONへ変換する。通常通知の空relatedMessageIdはSDKへundefinedで渡す。[OCの仕様・上限・検証](../../../crates/kbc-core/src/oc/docs/OC.md)。ノート・threadのURL削除と参加イベントの実OC網羅性は後続調査。

## 永続Volumeなしの復旧

mainはGitHubPersistence.restore / restoreSettingsの後にAuthStorageとCoreを開く。beforePersistがsequence予約とtoken退避を通信前に確定する。Coreのsnapshotは起動・毎分の変更時・正常終了で暗号化保存する。全て既存の非公開データrepoを使う。[関数と障害時の契約](../../../docs/operations/GITHUB_RECOVERY.md)。

受信metadataとsenderDisplayNameをProtocol v7へ正規化し、Profile更新をNAME、複数kickeeを個別Eventへ変換する。展開後のページは100件・256KiBに分割し、最後まで古いcheckpointを保つ。LogSyncは同じ追記先を4MiBまで5分ごとに更新する。競合はblob SHAで確認し、再取得してmergeする。`OC_LOGS_ENABLED=1`は旧データ変換後に設定する。[ログの契約](../../../crates/kbc-core/docs/LOGS.md)。

トーク補完のdrainChatはsubscription省略時に以前のトーク購読IDを保持し、未取得なら省略する。SDKのSquareChat.listenもトーク補完で購読IDを要求していない。返値にIDがある場合は正のsafe integerを検査する。accountのPUSH sign-on・lease検査は別で、省略可能にはしない。Smokeでは購読情報なしの初回・継続ページを受付し、checkpointと重複照合を確認する。

ApiScheduler.runはenqueue時のSendAttemptを保持し、pumpが別RPCの完了Contextから起動しても要求元のsendScopeへ戻して実行する。送信以外の要求ではsendScopeを空にし、別送信の開始を誤記録しない。2枠を別RPCで埋めた後の実SDK送信をSmokeで再現し、修正前は実transport例外をqueuedへ誤分類、修正後はsending記録とunknown確定を確認する。

## 投稿から独立した参加・退出の取得（v7）

`Receiver.run`はPUSH loopと補助取得loopを並行して監視する。`joinedChatPage`は起動時と10分ごと、snapshotのイベント100件×最大21ページ・2,048トークまで。通知設定のあるトークと有効な参加監視の本OCは3秒、その他はログ回収のため60秒を取得完了後の目安にする。初回を分散し、設定変更は次のloopで反映する。新しい参加トークの発見は最大10分待ち得る。

一覧を全ページ取得できた場合だけ、その一覧のトークへ定期取得を限定する。一覧にない旧通知設定は保存したまま対象から外し、再参加は次の一覧更新で戻す。初回の一覧未取得時は通知設定を使い、一覧失敗では前回の対象を維持する。`listedChats`は最後に取得できた一覧件数、`unlistedPriorityChats`はその一覧にない旧通知・監視設定の件数。個別MIDはhealthへ出さない。PUSHが要求したトーク補完は定期取得とは別に維持する。

`drainChat`は同じトークの取得Promiseを共有し、PUSH補完と定期取得がcursorを並行更新しない。全トーク合計2取得、1回4ページ・各100件、保存済みcontinuationから続ける。定期取得は期限の古い順。新着メッセージやPUSH hintなしでも取得・受付・通知配送が動く。参加者一覧の常時取得はしない。

失敗は対象トークだけ2秒〜15分のbackoff、一覧取得失敗は60秒後に再試行し、以前の対象を残す。Core保存失敗は全体停止。全通信は既存ApiSchedulerの2並列・250ms間隔・制限時cooldownを共有する。API回数は増えるため、3秒を全OCで保証せず、OC数・混雑・制限応答で周期を見直す。`pollingChats / priorityChats / pollCycles / pollPages / pollFailures / discoveryFailures / maxPollDelayMs`をhealthへ出す。pollCyclesは完了した周期、pollPagesは定期取得が開始したページで、別経路への合流はAPI回数に加算しない。

normalizeEventsのsourceはpush / chat / pollを区別する。MemberChangedにはeventTypeとreceivedAtMsも付け、ログのextraへ残す。イベント作成時刻と取得時刻を比較できるが、サーバーで発生してから保持されるまでの時間を保証しない。通常応答はrelatedMessageIdなし、副官部屋で原因投稿を残す一斉参加の監視などは同じOCのサブトークからreplyする。[ID取得と参照の範囲](../../../crates/kbc-core/src/oc/docs/ID.md)。

2026-10-04の実API照合: getJoinedSquareChatsはNOT_IMPLEMENTEDを返した。LINEJS 3.4.2のClient.fetchJoinedSquareChatsと同じく、fetchMyEventsの初期snapshotからnotifiedCreateSquareChatMember.chatを抽出するjoinedChatPageへ変更した。初回の空syncTokenから、syncToken・continuationToken・subscriptionIdを有限な一覧用cursor（base64url JSON / 2,048byte）で継続する。PUSH側のcheckpoint、SDK poll.sync、通常Commandの受付へこの一覧snapshotを渡さない。getJoinedSquareChatsへの再要求は廃止し、通知設定済みトークは一覧失敗時も独立取得する。snapshot一覧はOC設定のトーク選択・BOT管理者の!id talk ocでも共有する。

## 管理下OCのテスト操作（v8）

Inspectはトークcacheを更新し、対象Botの現在roleと最大2メンバーのOC・MIDを確認する。RolesはROLE属性とrevisionでupdateSquareMembersを呼び、返値の変更対象・役割を確認する。Postはメンション付き通常投稿、Deleteは管理者削除。deliverActionは各API名に対応したSendAttemptを使い、実fetch直前のsending・30秒期限・unknownの契約を共有する。対象許可・引数・previewはRustに置く。既存の2配送・2照会・全RPC枠を共有し、背景APIを追加しない。[OCテストの仕様と検証](../../../crates/kbc-core/src/oc/docs/TEST_OC.md)。

2026-10-04、利用者向け文面を[共通カタログ](../../../crates/kbc-core/docs/MESSAGES.md)へ分離した。Protocol v14のSticker.textはCoreで設定した代替文をSTKTXTへ渡すための項目。旧保存Actionは従来の代替文へ復元し、NativeとAdapterを同時に更新する。

## 公開参照データ（2026-10-05）

mainはSearchDataUpdater.runを独立taskとして開始し、90秒周期・同時1子プロセスで公開mainを確認する。子プロセスは90秒期限、各HTTPは20秒・4並列・10MiB。LINE API枠を使用しない。失敗はmetricsへ記録し受信を継続し、停止時はAbortSignalで子プロセスも終了する。CoreへはSDKオブジェクトや索引JSONを渡さず、原子的に更新するファイルpathとsearchDataLive=trueを渡す。Coreの利用時は120秒期限を検査する。[関数と検証](../../../scripts/docs/SEARCH_DATA.md)。

2026-10-05、Protocol v16のNativeと同時更新する。mainは既存のrunMediaJobsとともにrunStoreMonitorsを一度起動し、shutdownで取消・終了を待つ。ストア取得と検知はRust、通知の送信は既存deliverActionとApiScheduler。新しいLINE ReceiverやAPI Queueは追加しない。[詳細](../../../crates/kbc-core/src/store_update/docs/STORE_UPDATE.md)。

Protocol v17。deliverActionはthreadRootIdがある通常テキストをgetSquareThreadMid→300ms待機→sendSquareThreadMessageへ渡す。親送信後の1秒待機はCoreのdueへ保存する。照会のみ最大3回、送信開始後のエラーはunknownで自動再投稿しない。ApiSchedulerの共通枠・実fetch前のsending記録を共有し、本文の通常トークへのfallbackはない。smoke:skdで照会の準備遅延・送信境界・送信先とunknownを模擬検証する。[詳細と実OCの未確認点](../../../crates/kbc-core/src/skd/docs/SKD.md)。

## 複数メンション（v18、2026-10-05）

MessageMention.additionalの最大8件を先頭と合わせてMENTION.MENTIONEESへ変換する。Inspectは最大9メンバー、同じ既存枠で順番に取得し、所属を照合する。旧DTOのadditionalなしは以前の単独entryを維持する。同一範囲の複数entryが全対象の通知になるかは実機で確認し、Adapterの送信成功だけでは証明しない。[テスト仕様](../../../crates/kbc-core/src/oc/docs/TEST_MENTION.md)。


2026-10-08、Protocol v19のMembers.chatMembers=trueではgetSquareChatMembersを実行トーク・20人/ページで取得する。JOINEDと空queryに限定し、親OCと返却memberの所属を確認する。未指定は従来のsearchSquareMembers。判断・補完回数・名前照合・候補選択は[Core](../../../crates/kbc-core/src/oc/docs/ID.md)に置き、独自Queue・cache・常時巡回を追加しない。

## 2026-10-08：独立した履歴取得と小分け削除

Protocol v20のHistoryは採用LINEJS 3.4.2の生成Thrift定義でfetchSquareChatEventsへFORWARD/BACKWARD・inclusive・独立cursorを渡す。返すDTOは50イベント以内の件数とmessage ID / sender ID・投稿時刻。Protocol v21では参加者MID・参加イベント時刻も返す。notifiedJoinSquareChat、同じトークのJOINEDなnotifiedCreateSquareChatMember、同じOCのJOINEDなnotifiedCreateSquareMemberから取り出す。本文は渡さない。chatの親squareと各message.toを照合する。DeleteMessagesは同じsquareに属するchatへのdestroyMessagesで1〜20件の重複なしIDに限定。共通ApiSchedulerとsending境界を使い、構造化ILLEGAL_ARGUMENTだけfailedとしてCoreの件数縮小へ戻し、それ以外の通信後失敗はunknown。受信poll.sync・checkpoint・独自Queueは追加変更しない。[Coreの権限・走査状態・制約](../../../crates/kbc-core/src/oc/docs/REMOTE_MUTE.md)。

2026-10-08、履歴削除による他OCの照会待ちを抑えるため照会loopを1本から2本へ変更した。個々のPurgeは結果保存後に次Actionを一つだけ生成するため、同じ履歴cursorを並列取得しない。全RPC共通ApiSchedulerの2並列・250ms・待機32件は維持し、Coreでは履歴・一覧のPurge読み取りを通常照会の後に選ぶ。LINEへのAPI制限値を増やす変更ではない。
