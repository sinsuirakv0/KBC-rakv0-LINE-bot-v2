# Discordと同じ公開資料の検索索引

2026-10-05。schemaVersion 2。ut / tut / stはDiscord Bot v2の `02e6e9bbaeaabd93625a80056c372dc4f1733419` のDataSourceと同じ公開資料・mainを参照する。LINEアカウントやOCログは取得しない。

| リポジトリ | 資料 |
| --- | --- |
| sinsuirakv0/KBC-rakv0-assets | jp/sitedata/character-index.json、Data/unitbuy.csv、res/Enemyname.tsv、Map_Name.csv、分類別StageName |
| sinsuirakv0/KBC-rakv0-event | data/stage_type.csv、sale_name.csv |
| Sugar2550/omoroirie | data/enemyname.json、通常章・ゾンビ章のStageName |

本番は `SearchDataUpdater.run` が開始時と90秒ごとに `scripts/search-snapshot.cjs <SEARCH_DATA_PATH> --live` を1プロセスずつ起動する。既定の保存先は storage/search/catalog.json。PUSHやLINE APIの処理とは独立し、GitHub REST APIを定期利用しない。HTTPは最大4並列・1資料20秒・10MiB、プロセスは90秒以内。全資料を取得・検証できた場合だけ、tmpからrenameで索引を公開する。

`validatedAtMs` は取得開始時刻。Coreは確認から120秒以上経過した索引を使用せず、更新確認失敗の案内を返す。取得失敗は旧索引の時刻を延長しない。mainは searchDataLive=true を指定し、確認時刻のない旧同梱ファイルへのfallbackも禁止する。初回取得中もping・OC管理・受信を継続する。

Coreは検索・候補操作・素材準備時に索引を読み、処理終了時に破棄する。Runtimeは索引・正規化結果を常駐保持しない。通常会話・ping・OC設定には索引を読まない。SQLiteの検索Sessionには候補番号・条件・revisionだけを最大10分保存し、名称一覧は保存しない。検索データや参照先のrevisionが変わったSessionは操作時に失効し、再検索を案内する。確認時刻だけが変わった場合は同じSessionを維持する。

素材画像・motionもDiscordと同じassets/mainを参照する。素材の存在結果はメモリcacheに残さず、HTTP Clientと全体2枠だけを共有する。処理中の画像・描画bufferは実行に必要な間だけ保持し、検索索引の保持とは分ける。mainの更新が複数HTTP要求の途中に発生する場合、資料全体の同一commitは保証できない。

資料のETagと本文はディスクの catalog.json.sources/ に保存する。304はサーバーが変更なしと確認した場合のみ再利用し、通信失敗では再利用しない。資料は最大128件、変更で不要になった資料は成功周期後に削除する。子プロセス終了時にJSの取得本文・変換用データを全て解放する。Cache-Control: no-cache を付けるが、GitHub側で公開されるまでの遅延はBotの2分期限とは別。

手動の `npm run snapshot:search` はこれまで通り各mainの固定commitを解決し、再現用の data/search/catalog.json を生成する。このファイルはオフライン検証専用で、本番の定期更新を代替しない。Native直接呼出のsearchDataLive未指定はこの固定fixtureを許可する。

変換はID連続性、形態・別称、UnitBuy列61/62、敵の行番号・空欄、分類range・予約ID、sale別称、章3行、stage列番号・重複IDを検証する。R分類のStageNameを優先し404の時だけ通常分類へ進む。空欄・@を飛ばしても列番号を詰めない。

2026-10-05の公開main取得: ユニット882、敵792、マップ・ステージ7,748件、BOM込み2,104,594byte。entries SHA-256は `22967fd2b98705a139f432e75ec1b7d625a61e8533c22eb79dbc4b35afa5cb00`。更新で件数・内容は変わる。同梱の旧fixtureは876 / 788 / 7,704件のまま維持する。

[検索・Session仕様](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)、[更新ツールの責務・検証](../../../scripts/docs/SEARCH_DATA.md)。
