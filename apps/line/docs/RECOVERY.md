# 認証・接続復旧とAPI利用停止

2026-10-18。利用者指定により、通常処理を止めて最大3回の復旧を試し、失敗後は1分ごとに1回だけ復旧を確認する。停止はBot側の保護状態であり、LINEによるアカウント制限の確定を意味しない。Protocol v23を維持する。

## 状態と所有者

| 状態 | 処理 |
| --- | --- |
| ready | 通常の受信・照会・配送。初期値。Receiverのreceivingとは別で、起動中・再同期中のhealthは503 |
| api-recovering | 認証失敗またはPUSH全体の接続失敗で通常APIを止める。保存済みrefreshTokenで認証更新、保存完了待ち、PUSH再接続を最大3回。失敗間隔は2秒 |
| api-suspended | 3回失敗後。直前の試行失敗から60秒待って1回だけ更新・再接続する。失敗なら再び60秒待つ。監視中に高速3回試行へ戻さない |

復旧の所有者は`Receiver.runPush`の1本だけ。起動時にSDK loginが認証で失敗した場合は、Coreを作る前のmainのlogin loopが同じLineRecoveryを使い、その後Receiverへ渡す。loginと最初のPUSHは同じ準備済み試行として扱い、refreshを二重に呼ばない。

HTTP接続・ping・refresh成功だけでは再開しない。service 3のsign-onの正常応答、イベント配列・syncToken・正のsubscription IDを確認する。保存済みcontinuationがあればその読み取りを優先し、最初のページをCoreへ永続受付してから通常APIを戻す。新しいoriginへ変更したり、checkpoint・pendingChats・Outbox・認証情報を削除したりしない。

## 関数の働きと関係

| 関数 | 責務 |
| --- | --- |
| authenticationFailed | SDKの構造化認証code / 明示codeを判定。AUTHENTICATION_FAILURE、AUTHENTICATION_FAILED、MUST_REFRESH_V3_TOKEN、NOT_AUTHORIZED_DEVICE、401。エラー本文は解析しない。OC個別のNOT_AUTHORIZED・FORBIDDEN・NOT_FOUNDを全体認証失敗にしない |
| ApiScheduler.run → LineRecovery.notice | 認証失敗を1件の停止へ集約。共有キューの未通信RPCをLineApiUnavailableで取り消し、現在のPUSHを閉じる。新しい通常RPC・実fetch・PUSHの開始も防ぐ |
| LineRecovery.beforeConnect / failed | 復旧回数・次回時刻・監視回数を有限な状態で管理。beforeConnectが更新準備を行い、失敗はfailedへ戻す。refreshTokenがない場合、または更新だけが拒否されても現在のaccessTokenが残っている場合は、そのtokenで購読を確認する |
| client.auth.tryRefreshTokenのラッパー | SDK内部の自動refreshも同じ実行中Promiseへ合流させ、同時rotationを避ける。SDKが新しいaccessToken / refreshToken / expireを保存した後にAuthStorage.flushを待つ。token値はログへ出さない |
| ApiScheduler.withProbe | 復旧所有者による更新・初期購読と最初のページ受付だけを停止中に許可する。別のRPCの完了Contextから起動しても、enqueue時のprobeScopeを保持する。共通の2枠・250ms・cooldownを使い、新しい待機列は作らない |
| Receiver.session → LineRecovery.healthy | その試行と異なる停止通知が途中で発生していないことも確認し、正常な最初のページの保存後に再開。古い接続の成功で新しい停止を解除しない |
| ApiScheduler.waitReady | 補助取得loopとmainの配送・照会loopが同じ再開Promiseを待つ。配送loopはAction取得前後で確認。abortで待機を解除し、Timer・Workerを追加しない |

SDKの自動更新はMUST_REFRESH_V3_TOKENだけを対象とするため、SquareのAUTHENTICATION_FAILUREにはこの明示的な復旧経路が必要。SDKの[採用版](LINEJS_VERSION.md)は3.5.0。

通信開始済みのRPCは完了・既存の15秒取消を待つ。未通信の書込Actionは従来どおりOutboxへ再待機、通信開始済みの送信・処分がunknownなら自動再送しない。既に開始扱いになった読み取りは失敗で確定する場合がある。処分の30秒受付期限、SessionのTTL、API cooldownは停止中も延長しない。Core保存・認証保存の障害は監視で隠さず、従来の全体停止へ伝える。

PUSH以外のOC個別エラーは局所backoffを維持する。HTTP 429等は既存cooldownを尊重する。停止中もLINE APIを使わない公開データ更新・ログ/GitHub退避・metricsを継続する。起動し直すと復旧カウンターは初期化されるが、checkpoint・Outboxは既存snapshotから復元する。

## 観測と検証

stdoutの`line-recovery`、毎分metricsとhealthの`recovery`にstate / attempts / maxAttempts / checks / nextAttemptAtMs / codeを出す。receiverStateもapi-recovering / api-suspendedへなり、healthは503。再開後は保存済みcheckpointから追いついてreceivingになると200。個人MID・本文・生の例外・認証値は追加しない。

`npm run smoke:recovery`は、SDKの実tryRefreshTokenのrotation保存と同時更新の合流、共有枠の完了Contextをまたぐprobe、3回失敗、通常API/配送の停止、1周期後の監視、refresh拒否後の既存tokenによる購読、購読応答が来るまで非再開、checkpoint維持、未送信Actionの1回配送、OC個別FORBIDDENの非停止、abortを検証する。待機周期だけテストで短縮し、本番の2秒/60秒は固定。実LINEでの制限解除や認証復旧の成功は別途運用で確認する。

検証結果: TypeScript check/build、smoke:recovery、smoke、smoke:oc、smoke:skd、smoke:persistenceが成功。資料リンク・日本語ファイルのBOM・diff検査も実施。Rust/Protocolは変更しておらず、既存のProtocol v23 Nativeで検証した。本番配備・実LINEの認証更新や自動再開は未実施。
