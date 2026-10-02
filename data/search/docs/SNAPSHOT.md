# Discordと同じ公開資料の検索索引

2026-10-03。schemaVersion 2。旧LINEの検索データを使わず、Discord Bot v2の `02e6e9bbaeaabd93625a80056c372dc4f1733419` にある ut / tut / st のDataSourceを基準に生成する。OCログ・認証・個人データを含まない。

| 公開リポジトリ | 固定commit | 用途 |
| --- | --- | --- |
| sinsuirakv0/KBC-rakv0-assets | 4fce6ec9ab71cad90c5cb5d0a28d9c6364a3de9a | character-index.json、Data/unitbuy.csv、res/Enemyname.tsv、Map_Name.csv、分類別StageName |
| sinsuirakv0/KBC-rakv0-event | 802bae572730cccb46cc5f5b3939627b3d7d7aba | stage_type.csv、sale_name.csv |
| Sugar2550/omoroirie | 7fabdb46d11ec3e5fa22cc024806c5664ae01fbc | 敵の別称、通常章・ゾンビ章のStageName |

索引はユニット876、敵788、マップ・ステージ7,704件、BOM込み2,092,160byte。entriesのSHA-256は `b77533d12bd4e9366555df3423dde5869d495884dbfea294662135544a2b1648`。Coreはファイル全体の指紋をSessionのrevisionにし、変更した索引を以前の候補番号へ適用しない。

`node scripts/search-snapshot.cjs` で各mainのcommitを確定してから最大4並列・20秒・資料ごと10MiBで取得する。全資料の検証が通ってからtmpをrenameし、取得・検証失敗では現行snapshotを残す。再生成したcatalogとsourceの差分を確認し、commit・再配備する。Bot起動・検索時の名前データ取得、自動更新は行わない。素材画像・モーションもsnapshotのassets commitに固定するため、UnitBuyの共有形態と画像の版がずれない。

変換時はユニットIDの連続性、1〜4形態・別称、UnitBuy列61/62、敵の行番号と空欄、分類rangeの重複・予約ID、マップのsale別称、章の3行、stage列番号、重複IDを検証する。StageName_R分類_ja.csvを優先し、404の時だけStageName_分類_ja.csvへ進む。空欄・@を飛ばしても列番号は詰めない。

Runtime索引には名称・別称・形態数・共有素材ID・表示ID・正規のraw/typeキー・JDBリンクだけを残す。説明文や元の巨大CSVを同梱しない。[検索仕様](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)。
