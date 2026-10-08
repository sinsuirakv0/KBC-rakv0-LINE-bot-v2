# IDとリプライ参照

2026-10-04。実装・オフライン検証済み。Protocol v7を既存Northflankへ配備し、参加一覧の実API取得を確認した。ID各形式の実LINE表示は運用観測を続ける。旧LINE src/commands/id.tsを参照し、照会を共通OcRequest / Jobへ移した。通常のprefixは!、o.も受け付ける。

!idは自分、メンション・p MID指定は対象、talkは現在トーク・親OC、ocは親OC MIDを表示する。talk ocはBOT admin以上の参加中OC一覧。snapshotのイベント1ページ30件から参加トークを表示し、続きは--cursor。OC管理者権限だけでは全参加OC一覧を許可しない。同じOC以外のmember照会結果は表示しない。

名前検索は同じOCの保存済み名前と現在のLINE member directoryを照合する。NFKC・小文字・空白除去・部分一致と順序を保った文字一致を使う。oldはLEFT / KICK_OUT / BANNED / JOINEDを調べ、退会済みも表示する。状態ごとに20件×最大4ページ、候補20人まで。上限では絞り込みを案内し、全参加者の無制限な取得を持ち込まない。末尾logは状態数・ページ数・表示件数だけを表示し、生のSDK例外や巨大なdebug payloadを出さない。旧log allの過去履歴一括取得は利用者指定で後回し。現在の定期取得・ログ保存とは別扱い。

message / reply / metadataはmessage IDとrelatedMessageId / relatedMessageServiceCode=SQUARE / messageRelationType=REPLYを取得する。replyは入力のリプライ先、messageは入力自身が既定。message IDを引数で直接指定でき、--chat mMIDで受信済み参照を絞れる。元トーク・OC・送信者・時刻と元投稿のreply metadataは同じOCの観測済み情報だけを表示する。未観測の元トークを現在トークと推定しない。

message_refsは本文なし、最大8,192件。参照可能なのは48時間以内。同じOCのサブトークのmessage IDも索引で参照できる。旧idはreply先を見ておらず、この部分は新版の追加。LINEJS 3.4.2のUnresolvedMessageはIDだけを持ち、OCの単一message IDから本文・元トークを取得するAPIはこの移植で確認していない。未観測の古い投稿を完全解決できるとは扱わない。別OCへのリプライ送信は未確認。

| 関数 | 働きと関係 |
| --- | --- |
| initialize / remember | 共通受付transactionでmessage_refsへ必要なID metadataだけを保存。既存の本文ログを複製しない |
| parse / execute | 入力・権限・表示範囲をRustで判断。OC contextと共通requestを使用 |
| next_member / search_page / complete | Member・Members・JoinedChatsを既存照会Workerへ渡し、有限ページと結果を永続Jobで継続 |
| message_info | 同じOCと任意chatに絞った索引照会。受信済みID・未観測を区別 |
| decorations / decoration_info / message_reference | スタンプ・LINE絵文字の必要項目を有限保存し、共通範囲の索引から専用ID表示を生成 |
| SquareDirectory.execute | SDK呼出とplain DTO変換だけ。共通API枠・cooldown・timeoutを共有 |

Protocol v7でmembers・joinedChatsのread DTOを追加し、NativeとAdapterを同時更新する。未完了の旧v6 Jobには新しい任意状態をdefaultで補い、旧OcChatに親OCフィールドがない場合も復元可能にした。既存Smokeで通常応答が非リプライであることと、別サブトークの入力をreplyまたはID引数で参照できることを確認する。

参加一覧のSDK入出力はjoinedChatPageを共有する。getJoinedSquareChatsが本環境でNOT_IMPLEMENTEDとなったため、LINEJS自身と同じfetchMyEventsの一覧snapshotへ変更した。通常受信のcheckpointは上書きしない。[取得の関数・上限](../../../../../apps/line/docs/ADAPTER.md)。

!id replyは情報取得。取得したIDでBOTから実際に返信する試験はBOT管理者専用の [!test reply](TEST_REPLY.md) を使う。--chatには元メッセージがあるトークMIDを指定し、投稿先は実行トークに固定する。別OCの投稿の参照可否も試せる。

## スタンプ・LINE絵文字のID

!id sticker（stampも可）は対象スタンプへのリプライでSTKPKGID・STKID・STKVER・STKOPTを表示する。!id emojiは対象投稿へのリプライ、またはコマンド本文に添えたLINE絵文字からproductId・sticonId・version・resourceType・UTF-16位置を表示する。sticker / emojiもmessage IDと--chatを受け付け、同じOCの観測済み情報だけを参照する。Unicode絵文字にLINEのセットIDを割り当てない。

取得したスタンプIDの送信試験はBOT管理者専用の[!test sticker](TEST_STICKER.md)を使う。LINE絵文字のproductId / sticonIdをSTKPKGID / STKIDと混同しない。

remember → decorationsが受信contentMetadataのSTK項目とREPLACE.sticon.resourcesからIDに必要な項目だけをmessage_refsへ保存する。本文・REPLACE全文は複製しない。各IDは64byte、REPLACE解析は32KiB、絵文字は先頭20個まで。message_referenceはmessage / reply / sticker / emojiのOC・トーク・48時間境界を共有し、decoration_infoが通常投稿として整形する。既存保存データの追加項目はdefaultで復元できるが、旧受信分の装飾情報はさかのぼって再取得しない。新たに投稿してから参照する。専用API・巡回・常駐索引は追加しない。

## 2026-10-08：メンションなしの名前検索と候補選択

`!id 名前` / `o.id 名前` はメンション不要。旧LINE c6796e0のid.tsでも名前だけで検索し、1件ならID、複数なら表示一覧を返していた。新版に既存の名前検索はあったが、複数人の番号選択はなかったため今回追加した。メンション・pMIDによる直接指定は維持する。

0件は見つからない案内、1件は通常のID表示、複数なら10人ずつの一覧へ番号1〜10をリプライする。次・前・3p・終了は共通Paginationの入力を使う（候補上限20人なので最大2ページ）。一覧には名前・MID・状態を載せ、名前は32 UTF-16単位へ短縮する。選択後は検索結果時点の名前・MID・状態を通常投稿で表示し、メンション通知は付けない。oldの複数一致にも同じ操作を適用する。

`id::complete → selection::start` が検索終了時に候補を共通sessionsへ保存する。`id_selection.rs`のpageは文面キーid.choice / id.choicesと共通navigationを使う。保存はrevision=id-v1、最大20候補・64KiB・共通128 Session・1本人×1トーク1件・送信成功から10分。専用table・Queue・監視・APIは追加しない。

`commands::sessions::apply_inner → oc::select_id → selection::select` が本人・トーク・最新prompt・期限を照合した後にページ移動／番号選択する。ID Sessionには検索素材snapshotを読み込まない。ページ変更はpending_payloadへ書き、Runtime::complete_actionの送信成功で確定し、旧一覧を管理者削除する。通信前の確定失敗は旧一覧へ戻り、unknownは照合まで切替待ち。SQLite snapshotで再起動後も継続する。名前検索時の公開・LINE照会の有限上限は変更しない。

2026-10-08、既存OC Smokeに16人のメンションなし候補を追加し、1件の直接表示・10人表示・番号選択・2p/次・範囲外・別人・通常会話の番号・送信失敗での旧ページ復帰・切替中の待機・成功後の旧一覧削除・再起動後の選択・古いpromptの無視・終了・非メンション送信を確認した。build、型検査、Clippy全target、fmt、文面393キー／470呼出、Command・文面・受信基盤のSmokeも通過。LINE通信は0件。LINEJS公開版はnpm registryで3.4.2を再確認し変更なし。実LINEでの名前候補表示と削除権限差は未確認。


## 2026-10-08：部分名で0件になる報告への補完

利用者は「健康おじさん」に対し「健」「おじ」で0件と報告。Coreの部分一致はこの両方を受理するが、旧id.tsのgetMembersによる一覧照合・searchSquareMembersの空displayName補完が移植時に欠落し、名前フィルタの結果だけに依存していた。また投稿から得たlog_membersの名前は状態が空文字のまま保存され、JOINED限定のキャッシュ照合から除外されていた。実LINEが0件を返した直接の理由は未確認で、APIの非公開マッチ仕様や利用者の入力ミスと断定しない。

search_pageは名前指定を先に使い、completeで候補が0件なら、JOINEDは実行トークのgetSquareChatMembersで取得した一覧をCore内のmatches_nameで照合する。oldのLEFT / KICK_OUT / BANNEDは空displayNameのOCディレクトリで補完する。Lookup.fallbackとscannedはdefault付きで永続Jobへ追加し、旧continuationを復元できる。新しいWorkerは追加しない。Protocol v19のMembersへ任意chatMembersを追加し、AdapterはtrueのときだけgetSquareChatMembersを使う。旧保存要求では未指定なので従来の名前検索へ復元する。照会先トークの親OCと各返却memberの所属を検査し、別OCの結果を混ぜない。名前指定と補完を合計して状態ごとに20人×最大4取得（通常は合計80人、初回0件なら補完は最大60人）、候補上限20人を維持する。oldは各状態で同じ上限。全件を無制限に取得せず、上限で続きがある場合は従来の絞り込み案内を付ける。まだ取得していない範囲の参加者まで見つけられる保証はない。

保存名はJOINED / 数値2 / 状態未取得を通常検索の候補にする。既知のLEFT / KICK_OUT / BANNEDはoldのみ。状態未取得をJOINEDに書き換えず、一覧と詳細は共通の「未取得」文面で表示する。候補の状態は検索時点の観測で、現在の参加を保証しない。処分コマンドの現在所属・権限確認は変更しない。

末尾logではAPIから取得した件数も表示し、0件取得と取得後の照合結果を区別できるようにした。これは延べ取得件数で、状態やページを跨いだユニーク人数ではない。2026-10-08、OC Smokeで「健」「おじ」の名前フィルタを0件にし、getSquareChatMembersを実行トークへ20人指定で呼んで「健康おじさん」を解決する経路を確認した。状態未取得の保存名も候補にし、状態を未取得として表示することを確認。build、型検査、Clippy全target、fmt、文面393キー／472呼出、OC・Command・文面・保存復元・受信基盤のSmokeは通過。実LINE通信0件、本番の該当アカウントでの返値・常駐負荷は未検証。

## 2026-10-08：talkIDによる遠隔メンバー検索

`!id 名前 talkID:mMID` / `o.id 名前 talkID:mMID`は指定トークの親OCで検索・候補選択。pMID直接指定も可。別トークの利用はBOT管理者専用。検索・参加者一覧補完のAPI先だけ切り替え、返信・Sessionは実行トークに固定する。[共通の対象解決・権限](REMOTE_MUTE.md)。
