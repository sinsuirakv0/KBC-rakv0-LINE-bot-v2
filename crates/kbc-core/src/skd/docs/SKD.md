# スケジュール差分とスレッド通知

2026-10-05。Discord v2の`commands/skd`、共通Schedule Model、gatya / sale / itemの名前解決を移植。旧LINEの`src/eventPush/squareThread.ts`とLINEJS 3.4.2のSquare Serviceを確認した。LINEJSの最新公開版は実装開始時にnpmで再確認し、3.4.2を継続した。

## コマンドと表示

`!skd` / `o.skd`は保存済みの最新TSV更新から追加・期間／必要版の変更を表示する。日付は`!skd 2026/10/05`または`!skd 2026 10 05`。JSTの日付が最も近い更新を選び、距離が同じなら前の日、同日なら新しい保存時刻を選ぶ。通常利用者も利用できる。

親は「イベントスケジュール\nリークを含むためスレッドに送信」。送信結果のmessage IDを保存し、1秒後以降に本文をスレッドへ送る。本文はプレーンテキストで、コードブロック・Markdownの太字・メンションを付けない。見出し、期間、IDと名前、変更内容、関連履歴URLを含む。ガチャの確定・step up・福引等、常設、ミッション5件と残りIDの表示はDiscord由来。

案内・差分ラベル・エラーは`content/messages/skd.txt`で編集できる。helpは`content/help/skd.txt`、全体indexは手動更新。生のTSV全行を表示するコマンドではなく、選択した更新の差分を表示する。

## 取得と更新検知

履歴は`sinsuirakv0/KBC-rakv0-event`の`raw/{gatya,sale,item}_{unix}.tsv`。GitHub mainのSHAを確定してからtreeと前後TSVを同じSHAで取得する。100秒以内の保存を一つの更新にまとめる。名前はDiscordと同じeventのCSV・cardsetting、assetsのシリーズ対応表・ステージ／ミッション名を使う。略称が取得できなければ通常名から導いたシリーズ名へ戻す。

`!pushsetting skd [on|off|status]`で実行トークの通知を設定する。onは省略できる。OC管理者、BOT管理者・モデレーターに許可し、副官のみでは許可しない。Android/iOSと同じ登録table・512件の全体上限、権限確認を共有する。`!pushsetting status`は3種類の状態を返す。

Discordの外部webhookとは異なり、LINEの既存監視workerから公開`state/skd-notifications.json`を60秒ごとに確認する。登録なしではHTTPを呼ばない。変化時だけ履歴・本文を取得し、公開ハッシュと固定SHA側のstateの一致、取得後の再確認を要求する。GitHubのmainとrawの反映時差で古いハッシュと新しい履歴を混ぜて基準を進めない。初回は最新ファイル時刻を基準にして無通知。

保存したcursor以降の履歴を古い順に最大4更新ずつ追跡する。同じ100秒の組へ後から加わった種別も、未通知ファイルだけ比較する。最後に扱ったファイル時刻をcursorにし、追跡途中はハッシュを未確定として次回も続ける。停止中の複数更新を最後の1件へ潰さない。

HTTP Client、同時2枠、15秒timeout、1応答4MiB上限はAssetServiceを共有する。履歴treeはtruncatedを拒否し最大100,000 entries、TSV最大20,000行、ガチャ128件／行、TimeBlock128件／行、値1,000件。辞書・tree・TSVは処理後に破棄し、常駐キャッシュを作らない。履歴APIだけ既存`PUSH_SUBSCRIPTIONS_GITHUB_TOKEN`を認証に再利用し、`GITHUB_TOKEN`があればそちらを優先する。tokenをraw等の別hostへ送らない。認証なしではGitHubの匿名API制限があるため、多人数による連続`skd`実行は取得エラーになり得る。

取得失敗・容量不足では60/120/300秒へbackoff。失敗でcursorを進めない。監視停止は既存CancellationTokenでHTTP・待機を中断する。

## 保存・配送

`store_versions`のskd行はハッシュとcursorのJSON、観測時刻・エラーを保持する。`schedule_updates`は最大16件の本文、`schedule_targets`はその時点の登録先（最大16×512件）を持つ。更新本文・対象・cursorは同じtransactionで保存する。Outboxが満杯でも永続対象へ保持できた更新は検知済みにする。16件上限／保存失敗ではrollbackし、後で履歴から再取得する。

配送は1回最大64対象、既存Outboxの空き枠へ順に展開する。親の本文part数も2,048枠の容量に予約する。準備中のjobは最大32part＋親の33枠を保守的に予約する（既存Media準備も同じ上限計算）。1スレッド最大32メッセージ、各1,500 UTF-16単位。超過時は短縮して黙って捨てず取得を失敗扱いにする。

親の成功と、1秒後の各本文Actionの作成は同じ結果transaction。unknownの親では本文を作らず、既存の照合で実送信IDを確定した場合にだけ続行する。親の確定失敗では本文を送らない。各本文のunknownは次のpartを停止させるが、別の通常返信を止めない。確定failedなら残りの未送信partも停止する。

Adapterは`getSquareThreadMid(chatMid, rootMessageId)`を読み取りだけ最大3回照会し、NOT_FOUND等の再試行間隔と照会成功後に300ms待つ。自分で送った親へのjoinは行わない。`sendSquareThreadMessage`はreqSeq、SQUARE_THREAD、thread MIDを渡し、共通ApiSchedulerの実fetch直前にsendingを記録する。通信開始後のエラー／ID欠落はunknown、自動再投稿しない。スレッド本文を通常トークへ送るfallbackはない。

OFFでは未配送の対象とqueuedの通知を取消す。送信中だった親が後から成功しても、解除済みなら本文を作らない。既にsending / unknownの結果は照合する。設定・未配送本文・対象・Outboxは既存の暗号化GitHub DB snapshotで復元する。遠隔退避以前に送信された可能性の扱いも既存の[復旧契約](../../../../../docs/operations/GITHUB_RECOVERY.md)と同じ。

## 関数の接続

| 関数・型 | 働き |
| --- | --- |
| commands::prepare / skd::parse_date | prefixと日付を検査し、共通MediaJob::Scheduleへ渡す |
| media_loop / prepare_schedule / finish_media | 既存の有限準備workerで取得し、本文を持つ親SendMessageへ置換。通常配送から独立 |
| SkdDataSource::load / build_update | 固定SHA、履歴選択、前後TSV、parser→diff→Metadata→formatter |
| updates_after / updates_since | cursorより新しいファイルを選び、停止中・同じ組の追加を追跡 |
| schedule_loop / poll_schedule | 既存run_store_monitorsの3つ目のLoop。共通HTTPでハッシュ確認・backoff |
| observe_schedule / dispatch_schedules | 更新・通知先・cursorの永続化と、配送枠への展開 |
| skd::complete / pending_count | 親成功から遅延本文を登録、失敗時の後続停止、予約を含む容量計算 |
| deliverAction / resolveThread | スレッド照会とLINEJS送信、実fetch境界・結果記録 |

## 検証と未確認点

Rustは日時入力、追加と期間変更、プレーンテキスト、履歴の日付選択・同じ組の追加、親unknown、再起動・due、本文の順序、通常返信の継続、多トークfanout・容量と復元を確認する。AdapterのsmokeはNOT_FOUNDからの照会成功、reqSeq／送信先、送信境界、通信後unknown、照会失敗で通常投稿しないことを確認する。公開sourceはignore付きの単発確認で、LINEへ送信しない。

実OCのスレッド送信・待機1秒の十分性、多OCのLINE API制限と配信遅延は配備後の確認事項。変更なしの確認でもGitHub CDNの反映遅延が加わる。メッセージを削除済み・スレッド利用不可等のOCでは、親だけ残って本文が配送できない場合がある。

2026-10-05の検証結果: Core全8件の通常テスト、SKDの公開source単発1件（3種別の辞書も取得）、workspace/all-targetsのClippy警告拒否、Native/TypeScript build、smoke:skd・commands・oc・messages・persistence・受信基盤のsmokeが通過。Nativeのo.skd受付→共有準備worker→親Action→本文Actionも、LINE通信0件の単発確認で通過し、待機は約1.03秒、本文3メッセージだった。公開データの観測値であり、実OCでの送信成功の証明ではない。

2026-10-06、!botの停止設定を予定登録と段階配送の両方で確認する。停止先は新しいschedule_targetsへ加えず、全宛先停止中は更新本文を待機tableへ追加しない。配信先のない更新はobserve_scheduleの同じtransactionで整理し、検知watermarkだけを進める。停止中の更新を再開後に送らず、本文だけの孤立rowで上限16件を消費しない。既存targetsは1周最大64件の配送処理でも停止を再確認し、見送り先を除去する。変更前からOutboxへ登録済みのActionは従来の確定・unknown契約を保つ。[BOT停止と権限](../../oc/docs/BOT.md)。

2026-10-07、model / metadata / labelsをgatya・sale・itemの参照Commandと共用した。JSONのrawは表示に使用し、Serializeから除くため既存差分の比較対象は増やさない。任意の略称は404だけを通常名へ戻し、その他の取得失敗は既存の失敗経路へ返す。[参照Command](../../event_data/docs/COMMANDS.md)。
