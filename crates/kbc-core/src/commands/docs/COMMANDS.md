# LINEのtxt・Discord検索コマンド

2026-10-03。実装・オフライン検証と公開素材の生成実験を完了。実LINE送信・Northflank負荷は未確認。

## 入力と移植元

`!ping` 等は [txt catalog](../../../../../content/docs/CONTENT.md) で登録。`!help / !ut help` は同じcatalogの案内を返す。`ut / tut / st` はDiscord v2のcommit `02e6e9b` の仕様・公開資料・素材解決を参照する。LINE向けの表示・Session・配送を共通化し、Discord Client・Reaction・編集は持ち込まない。

| 入力 | 動作 |
| --- | --- |
| !ut / !tut / !st | JDB検索ページ |
| !ut 0 / !ut ネコ | 正式な形態名と別称を検索。ヒットした形態・別称を表示 |
| !ut 検索語 -f | 正規化なし・正式な形態名だけを検索。-forceはutのflagではない |
| !tut わんこ / !tut 0 | 現行Enemynameの名称と別称。ダミーは対応する別称を表示 |
| !tut 検索語 -f / -force | 表記そのままで名称・別称を検索。数値ID解決は有効 |
| !st N000-000 / !st 3000-000 | 分類・数値IDを完全解決。省略した0埋めも数値として解釈 |
| !st -f 検索語 | 先頭のflagでID解決・正規化をせず名前だけを検索 |
| !ut 0 origin [icon/wide/sprite] [f/c/s/u] | 形態画像。共有形態はUnitBuy列61/62から素材ID・m suffixを解決 |
| !ut 0 origin gacha [m/z] | ガチャ画像。省略時f |
| !tut 0 origin | 敵アイコン |
| !ut 0 file [f/c/s/u] / !tut 0 file | 存在確認済みの画像・imgcut・mamodel・maanimを番号で選択 |
| !ut 0 motion png f a 0 | 攻撃のFrame 0をPNG生成 |
| !ut 0 motion mp4 f w 0~~5 a 0~~5 | 歩行6＋攻撃6FrameをMP4生成 |
| !tut 0 motion gif a | 敵の攻撃全FrameをGIF生成 |
| !test-notify 5 | 別の入力なしで期限通知が配送される確認 |
| !bot name 名前 | BOT管理者限定で実行OCのBotの表示名を変更。[仕様・関数・更新API](../../oc/docs/BOT.md) |
| !test reply メッセージID [--chat 返信元トークMID] 本文 | BOT管理者限定。実行トークへ返信し、別OCの投稿も参照試験できる |

prefixは `!` を既定とし、`o.` も受け付ける。コマンド名はASCII大文字小文字を区別しない。`unit / enemy / stage` は別名。検索引数は16語・512byteまで。通常はDiscordと同じNFKC・小文字・かな・長音・波線の正規化。空白区切りの語は一つの名称内でAND照合する。形態名と別称を連結して語を跨がせない。utの通常検索は正式形態名を優先し、その後に別称を調べる。

数値IDはDiscordと同じASCII数字。stはrawキーまたは最長分類名を使うtypeキーへ変換し、完全一致で1件を返す。IDで解決したマップはJDBのid=rawを使い、名前検索はtype/map、ステージはstage付きリンクを使う。マップ名とsale別称の両方で検索し、旧マップの表示名は維持する。[snapshotの元資料・更新](../../../../../data/search/docs/SNAPSHOT.md)。

motionはpng/mp4/gif、f/c/s/u（utだけ）、a=攻撃・w=歩行・i=待機・k=ノックバック。PNGはFrame番号省略0、動画は範囲省略・開始~~終了・開始 終了。複数segmentを順番に描画し、--fullで全体表示。[描画の関数・上限](../../motion/docs/MOTION.md)。

## LINEの表示と対話

通常1〜3件の詳細は1返信にまとめ、4件以上を8候補ずつ表示する。origin/file/motionの複数一致では先頭を勝手に選ばず本人の選択を待つ。最大512候補を保持し、総件数と絞り込み案内を出す。プレーンテキスト・空行・URLを使い、改行優先で1,500 UTF-16単位・1入力8 Actionまで分割する。

一覧への番号リプライは1〜8=選択、終了/取消/cancel=受付終了。ページ移動は一覧メッセージへの👍（NICE）=次、❤️（LOVE）=前という実リアクションで行い、9/0で移動しない。同じ本人・トーク・最新の実送信prompt ID・期限内のSessionへ限定する。別人、別トーク、古いprompt、通常会話の数字は無視する。Sessionは128件、1本人×1トーク1件、送信完了から10分。SQLiteに候補の索引・検索条件・file選択段階を残し、同じsnapshotなら再起動して操作を続けられる。

リアクション通知には実行者MIDがないため、稼働中の一覧・移動可能な方向だけ既存照会WorkerでgetMessageReactionsを取得し、検索者のMID・種類・時刻を照合する。表示名で本人と判断しない。最大4ページ×100件。同じ一覧の照会・配送中の連打はまとめ、常時ポーリングを増やさない。API失敗時は旧一覧を残して付け直しを案内する。[LINEJS仕様・追加APIと未確認点](../../../../../apps/line/docs/REACTIONS_AND_STICKERS.md)。

案内は一覧の長押し操作を明示し、👍 / ❤️の部分には標準LINE絵文字を埋め込む。commands::message_emojisが分割後の本文のUTF-16位置をProtocol v12のMessageEmojiへ渡し、AdapterがREPLACEを付ける。絵文字を返信してもページを変えない。本文画像と実リアクションは別のデータである。[採用ID・SDK構造と実表示の未確認点](../../../../../apps/line/docs/REACTIONS_AND_STICKERS.md)。

変更先はsessions.pending_payloadへ保存し、新一覧の送信成功後だけpayloadとpromptを入れ替える。切替中の番号には待機案内を返し、別ページの項目を選ばない。確定失敗は旧一覧を復帰し、unknownは照合まで切替待ちを保持する。一覧の各候補表示は96 UTF-16単位までに縮め、操作案内を同じ1,500単位以内の投稿へ収める。

編集の代替は新しい返信成功後の旧候補の管理者削除（square.destroyMessage）。送信完了時に10分後の清掃を登録し、操作時は同じ削除Actionを前倒しする。削除失敗で新promptを巻き戻さない。任意清掃を追加できないほどOutboxが満杯なら清掃だけを省略する。管理者権限のないOCでは一覧が残る場合がある。

画像・動画・ファイルは [共通Media Worker](../../../docs/MEDIA.md) が準備する。通常返信の配送枠は生成完了を待たない。LINEでは受付・完成だけを投稿し、描画進捗によるAPI連打・疑似編集を作らない。ファイル候補は最大39件のHEAD確認後に表示する。選択中に別検索へ進んだ場合、以前の確認結果で新Sessionを上書きしない。

## 関数と関係

| 関数・型 | 働きと後続 |
| --- | --- |
| ContentCatalog::load | txtを検証し応答・helpを起動時登録 |
| commands::prepare | prefix・alias・help・入力上限を判定。SearchCatalogへ検索を依頼 |
| SearchCatalog::load/search | snapshot検証・正規化索引とID索引を構築。matchした名称番号もSessionへ保存 |
| SearchCatalog::detail/page | ヒット形態・ダミー別称・ID解決によるURLと、検索/ファイルページをLINE向け整形 |
| origin_path/file_options/motion_plan | 共有形態IDと素材path・描画条件を解決。通信は行わない |
| sessions::apply/selected | 本人・トーク・prompt・期限を照合。選択・ページ更新・MediaRequestを受付transactionで保存 |
| sessions::request_reaction/complete_reaction | 現在の一覧だけ照会を登録し、検索者のリアクションを検証して未確定ページを保存。Runtime::complete_actionが送信成功後に確定 |
| Runtime::submit_batch | 検索をDB lockの外で計算し、重複排除・Session・軽量Action・MediaRequest・checkpointを同時commit |
| split_responses | LINEの文字数で分割。媒体のbyteをJSONへ入れない |

Bridgeの非同期受付は最大4受付・実処理1件。外部素材の取得や描画を受付・SQLite transaction内で待たない。基盤Smoke17条件、Command Smoke4経路、[実素材実験](../../../../../experiments/commands/docs/MEDIA_VERIFICATION.md)を確認した。コンテナ復元、実LINEでの番号返信・管理者削除・メディア表示は次の検証として残る。

## OC管理との接続

!ocの公開helpはContentCatalogを使う。管理入力・本人リプライ・審議・自動処理は [OC管理](../../oc/docs/OC.md) へ渡し、共通のsplit_responses・Outbox・prompt清掃を使う。OC照会は通常返信の配送枠を使わず、権限免除された投稿のCommandPlanは既存Runtime::apply_commandへ戻す。!コマンドの先頭語はURLとして誤検出しない。

通常応答・一覧・生成受付は入力へ自動replyせず普通に送信する。番号を入力する側は最新promptへのリプライを使う。!idのhelpは同じContentCatalog、取得・検索・同OCサブトークの返信情報は既存OC照会経路を使う。[ID仕様と関数](../../oc/docs/ID.md)。

利用者指定の実リプライ送信は!test replyへ追加した。本文は改行・空白を保って最大1,500 UTF-16単位、--chatには元メッセージがあるmから始まるトークMIDを指定する。投稿先は常に実行トーク。BOT adminだけが実行でき、既存Context照会・Outbox・SendMessageを共有する。通常の!replyコマンドや旧testの他機能は追加していない。[仕様・関数・試験範囲](../../oc/docs/TEST_REPLY.md)。

## 検証OCの操作

!test allowは検証OCのsMIDを複数登録・解除する。BOT adminだけに許可し、!test mention / delete / kick / deputy / adminは対象確認後、--apply付きで1回の実操作を登録する。--target-chatは対象トーク、!test replyの--chatは返信元。txtの案内・共通Context・既存Outboxを使う。[仕様・関数・上限](../../oc/docs/TEST_OC.md)。
