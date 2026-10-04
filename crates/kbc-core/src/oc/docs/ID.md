# IDとリプライ参照

2026-10-04。実装・オフライン検証済み。Protocol v7を既存Northflankへ配備し、参加一覧の実API取得を確認した。ID各形式の実LINE表示は運用観測を続ける。旧LINE src/commands/id.tsを参照し、照会を共通OcRequest / Jobへ移した。通常のprefixは!、o.も受け付ける。

!idは自分、メンション・p MID指定は対象、talkは現在トーク・親OC、ocは親OC MIDを表示する。talk ocはBOT admin以上の参加中OC一覧。snapshotのイベント1ページ30件から参加トークを表示し、続きは--cursor。OC管理者権限だけでは全参加OC一覧を許可しない。同じOC以外のmember照会結果は表示しない。

名前検索は同じOCの保存済み名前と現在のLINE member directoryを照合する。NFKC・小文字・空白除去・部分一致と順序を保った文字一致を使う。oldはLEFT / KICK_OUT / BANNED / JOINEDを調べ、退会済みも表示する。状態ごとに20件×最大4ページ、表示20人まで。上限では絞り込みを案内し、全参加者の無制限な取得を持ち込まない。末尾logは状態数・ページ数・表示件数だけを表示し、生のSDK例外や巨大なdebug payloadを出さない。旧log allの過去履歴一括取得は利用者指定で後回し。現在の定期取得・ログ保存とは別扱い。

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

remember → decorationsが受信contentMetadataのSTK項目とREPLACE.sticon.resourcesからIDに必要な項目だけをmessage_refsへ保存する。本文・REPLACE全文は複製しない。各IDは64byte、REPLACE解析は32KiB、絵文字は先頭20個まで。message_referenceはmessage / reply / sticker / emojiのOC・トーク・48時間境界を共有し、decoration_infoが通常投稿として整形する。既存保存データの追加項目はdefaultで復元できるが、旧受信分の装飾情報はさかのぼって再取得しない。新たに投稿してから参照する。専用API・巡回・常駐索引は追加しない。
