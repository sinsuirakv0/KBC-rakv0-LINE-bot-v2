# 遠隔メンバー検索・無通知ミュート・過去投稿削除

2026-10-08。実装・オフライン検証済み。実LINEでの履歴保持範囲・cursor進行・実削除は未確認。既存ミュートと旧LINE c6796e0のsquareHistoryProbe、採用LINEJS 3.4.2のThrift定義・SquareServiceを参照した。

## 入力・権限・送信先

- `!id 名前 talkID:mMID` / `o.id 名前 talkID:mMID`：指定トークの親OCで名前検索。pMIDによる直接照会、old、検索情報の末尾logも使える。
- `!oc mute userID:pMID 時間|inf|off talkID:mMID`：対象OC全体へミュートを登録・解除。
- `!oc mute list talkID:mMID`：対象OCのミュート一覧。
- `!oc mute alltalk userID:pMID [時間|inf] [talkID:mMID]`：ミュート登録と、対象OCの参加トーク全体で本人の過去投稿を管理者削除。時間省略は無期限。alltalkは解除・一覧と併用不可、対象は1人。

`talkID:`は1個のm+32桁hexだけ。sMIDはOC IDなので受け付けない。`!`と`o.`は同じ処理。IDのtalk / oc / message / reply / sticker / emoji / meなどには遠隔引数を追加しない。

実行トークと異なるトークの指定は、同じOCのサブトークを含めBOT admin（rank 2）だけ。実行元OC管理人・BOT modだけでは別トークへ照会・設定できない。実行トーク自身の指定は通常の権限を維持し、通常mute / alltalkはOC ADMINまたはBOT mod以上。Botが対象トークへ参加していること、対象memberのsquareが同じことを照合する。

検索の返信・候補Session・設定結果・削除の開始／終了結果は実行トークだけ。対象OCのmodroomへも遠隔操作のログメッセージを送らない。操作履歴は対象OCのSQLiteへ記録し、実行者は実行元の本人MID・名前のまま保持する。

遠隔で作ったMuteはsilent=trueを永続保存し、後日その人が本・サブトークへ投稿しても削除だけ行い、ミュート警告・メンションを送らない。旧データはsilent=false。通常ミュートを再登録すると従来の警告ありへ更新される。解除は同じOCの同じpMIDだけを削除する。

## 履歴削除の範囲と制約

ミュートを先に保存し、以後の新規投稿を既存moderation::muteが処理する。履歴削除はそのあとで開始する。履歴削除開始時にInspectでBotの現在所属・roleを改めて確認し、対象OC roleがADMINまたはCO_ADMINであることが必要。権限不足・履歴失敗でも登録済みミュートは残し、返信で伝える。

参加トーク一覧のJoinedChatsを30件ずつ取得し、同じsquareの本・サブトークだけを集める。アンカーのtalkIDは最初に含め、同じトークは重複させない。未参加トークは取得できない。スレッド内の投稿を列挙する処理は今回追加しない。

各トークは履歴用fetchSquareChatEventsをFORWARDから開始して現在位置を取得後、BACKWARD / inclusive=ONへ切り替える。syncTokenとcontinuationTokenはJobだけが保持し、Receiverのcheckpoint・poll.syncへ戻さない。受信Eventとして再投入せず、Command再実行・通知再送・ログ複製を起こさない。Coreに渡す情報はmessage ID・sender pMIDだけ。

1ページ50イベントから本人の投稿を選ぶ。本文や他人の投稿は削除Actionへ入れない。隣接ページの同じmessage IDは除外する。削除は20件ずつのdestroyMessages。構造化されたSquare例外のILLEGAL_ARGUMENTだけは明確な拒否としてfailedにし、そのバッチを10→5→2→1件へ縮小して同じ未削除候補を進める。縮小後の件数は後続ページにも使う。1件でも拒否されれば停止。通信切断・応答不明はunknownで停止し、自動再実行しない。他の管理操作のunknown規則は変えない。

同一OC128トーク、一覧と履歴を合わせて1,000ページ、元イベントから10分まで。履歴・候補は1ページ50件、削除Actionは20 ID、cursorは各2KiBまで。トーク一覧上限やcursor停滞では未取得の範囲を案内する。途中失敗なら以後のトークへも進まず、走査終了したトーク数・API成功件数・最後の未完了バッチ・取得済み未削除候補・API codeを実行元へ返す。

「走査終了」はLINEから取得できた履歴の終了を意味し、サーバーが返さない古い投稿の存在や実際の全件消去を保証しない。履歴取得・削除APIの実機での挙動を確認し、保持範囲やcursorが異なる場合はこの部分だけ見直す。

## 関数・状態・相互関係

| 関数 | 役割と後続処理 |
| --- | --- |
| remote::take_chat / prepare | talkIDを共通抽出し、元の実行者権限を確認。遠隔先Inspectを既存requestへ渡す |
| remote::complete / context / chat | 参加確認済みの対象OCを保持し、id::executeまたはcommands::executeへ戻す。元のeventとcontextは変更しない |
| commands::execute / target | 既存muteの対象Member照会・期限・容量・権限を共用。silentを保存後、alltalkならpurge::startへ |
| purge::start / next / complete | Inspect → JoinedChats → 各トークの履歴 → 本人の削除を既存Jobで順に継続。空ページで次トークへ進む |
| purge::delete_next / finish | 件数を制限して同じOutboxへ削除登録。成否・停止・上限を実行元へ通常投稿で返す |
| oc::request / complete | 対象APIのトークだけを切り替え、結果と次Actionを同じtransactionで保存 |
| SquareDirectory.execute / deliverAction | 所属範囲・DTO件数を検査。SDK通信とsending境界、明確な件数拒否の区別だけを担当 |
| moderation::mute | OC全体の既存ミュートを参照し、silent=trueなら後続警告を生成しない |

Phase::Remote / MuteInspect / MuteChats / MuteHistory / MuteDeleteと任意Job.remote / Job.purgeを保存する。旧Jobは任意状態なしとして復元可能。新しいDB table・Worker・Queue・定期巡回は追加しない。読み取りは1照会Worker、削除は既存配送Worker、共通API枠・cooldownを使う。queryingは再起動時queuedへ戻る。sendingの削除はunknownとして保持し、自動再送しない。

Protocol v20でHistory・DeleteMessagesとOcHistoryPage / OcHistoryMessageを追加。NativeとAdapterは同時更新する。

## 最小検証

既存smoke:ocへ追加し、実LINEには接続しない。遠隔検索と番号選択の実行元固定、参加者一覧補完の遠隔先、OC管理人・BOT modの拒否、遠隔ミュートの保存先／履歴／再起動後の警告抑止を確認する。全OCの一覧から他OCを除外し、サブトークを含む複数履歴ページ・境界重複・他人の投稿除外・20件からの縮小・unknown停止／再起動非再送・履歴失敗・削除権限不足をSDK mockで確認する。

検証結果: `npm run build:native`、`npx tsc`、`cargo clippy --locked --release -p kbc-core -p kbc-protocol --all-targets -- -D warnings`、`cargo fmt --all -- --check`、文面キー監査、`smoke:oc`・`smoke:messages`・`smoke`が成功。smoke:ocは既存経路とremote-id-mute-purgeを同時に確認した。実LINE接続・本番配備はこの変更では行っていない。
