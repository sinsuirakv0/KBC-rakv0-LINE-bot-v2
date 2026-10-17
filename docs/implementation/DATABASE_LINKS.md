# 検索リンクのKBC移行

ut / tut / st（LINEはunit / enemy / stageの別名も含む）の検索ページ・詳細は、初期状態で `https://kbc-rakv0.pages.dev` を使う。最後の独立引数が `jdb` の場合だけ従来のJDBへ切り替える。大文字小文字は区別しない。引数なしの `ut jdb` などはJDBの検索ページを返す。

`crates/kbc-core/src/database_links.rs` の `DatabaseSite::split_arguments` が末尾の指定を検索語・操作引数から取り除く。`search_url` が入口、`detail_url` が既存IDとJDB URLを受けて詳細を組み立てる。WebへURLkeyを発行・付加する処理はない。

味方は `/pages/unit-detail/?id=ID`、敵は `/pages/tunit-detail/?id=ID`。ステージは `/pages/stage-detail/?map=KEY` に、個別面なら `&stage=番号` を付ける。マップのリンクは全ステージ表示から開く。章のraw IDはEOC/ITF/COTCと各Zの1〜3へ変換し、EはEX、2_Invと2Z_InvはCOTC-IとCOTCZ-Iへ対応させる。面番号の先頭0は除き、0番は残す。JDB指定では従来のtype/map、ID指定のrawリンクを維持する。

DiscordはDetail操作またはStageSelectionにリンク先を保持し、数字リアクションで選んだ結果にも反映する。LINEはSearchSessionのsiteへ保存し、番号選択・ページ移動・再起動をまたいで保持する。旧Sessionでsiteがない場合はKBCを使う。検索カタログのJDB URLは互換入力として保持し、返信時にKBCへ変換する。snapshot更新処理による差戻しを防ぐため、生成データを一括書換えしない。画像・file・motionの取得元や処理は変更しない。

確認はkbc-coreのコンパイル、既存Command Smokeのリンク期待値、Nativeへの模擬入力。実Discord/LINEへの送信は行わない。KBC側の生成マップIDとも対応を確認する。

2026-10-17検証: release Nativeで入口6通り・通常/末尾jdb・章/EX/侵略の詳細18入力、再起動後の本人番号選択でJDBリンクの保持を確認。TypeScript検査と既存Command Smokeの4シナリオも通過。実LINEへの投稿は行っていない。
