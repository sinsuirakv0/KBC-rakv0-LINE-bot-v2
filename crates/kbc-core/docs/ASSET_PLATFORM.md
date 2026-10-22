# Assets保存先変更への対応

2026-10-22。公開assetsのmain（調査時db586c92df34ab54f4417a03fa1553a4faeac134）は旧jp/sitedataを廃止し、metadata/jp/current.json、generated/jp/datasets/<buildId>/、jp/Local・jp/server・overridesへ分離している。キャラ名だけでなく、敵・ステージ・ガチャ対応表・イベント名・画像・モーションも影響する。

## データ契約

currentのstatus=verified / schemaVersion=1 / sourceRegion=jp、datasetIdとbuildId、generatedPathを検査する。同じGit commitからmanifestとindexes/assets.jsonを読み、manifestのSHA-256、索引のサイズ/SHA-256とheaderを照合する。キャラ名はmanifestのsite/characters.json、それ以外はlogical pathに対応するfiles[path].pathを使う。

実ファイルも取得後にサイズ/SHA-256を照合する。元pathは同じdatasetIdのjp/Local、jp/server、overrides/jp/assetsの範囲に限定する。異なる版のServerや補完を推測で選ばない。BOMはハッシュ計算の前には除去せず、JSON/文字列解釈時だけ処理する。

## 共通関数と関係

| 関数 | 役割 |
| --- | --- |
| asset_platform::AssetPlatform.load | mainをGit commitへ固定し、current→manifest→採用元索引を1本に集約して読み込む。固定SHAではref照会不要。保持は全体で1 snapshot、利用期限120秒。期限後の更新失敗でstaleを返さない |
| Dataset.resolve | character-index.jsonをsite/characters.jsonへ、それ以外のlogical pathを採用された元pathへ解決。索引にない任意ファイルはNone。パストラバーサルを拒否する |
| Proof.verify | 取得した元のbyte列とdescriptorのsize / sha256を比較する |
| AssetService / HttpService | 既存Client・Semaphore・timeoutを使って共通resolverを呼ぶ。Command固有のHTTP ClientやQueueは追加しない |
| RemoteAssetSource.pinned / AssetService.pinned | motionの取得開始時にmainを固定SHAへ変える。sprite・imgcut・model・animationを同じ版から取得する |

既存のjp/sitedata形式の文字列は、呼出側のlogical path指定として残す。HTTP境界で新配置へ解決するため、これらの文字列を新しいmainの旧実体へ直接HTTP要求しない。ut/tut/st、sale/gatya/skd、既存motion代行も同じ経路を使う。

移行前の固定commitに限り、currentが404ならそのcommitの旧jp/sitedataを使う。保存済みジョブと既存の固定fixtureを保持するためで、mainの失敗や破損した新索引を旧配置へ切り戻す処理ではない。

## 上限と更新

調査時の採用元索引は53,903件・11,458,001 bytes。索引の取得だけ16MiB、件数131,072を上限にする。本文を同時に全件取得せず、保持するのはpath・サイズ・hashのmetadataだけ。通常ファイルの上限はLINEの4MiB / Discordの既定8MiBを変更しない。共通HTTP枠もLINE 2 / Discord既定4を維持する。

Discordの既存Commandごとのmetadata cacheとSession方針は維持する。新resolver自身の保持期限は120秒で、添付本体をcacheしない。LINEの検索更新は短命Nodeプロセスがassetsのcommit・current・索引を固定して読み、最新性120秒の既存catalogへ変換する。buildIdもcatalog revisionへ含め、データ版の変更時に古い番号候補を再解釈しない。fileの選択案内には、準備時に解決した実体URLを保存する。

mainの固定にGitHub APIを使う。GITHUB_TOKENを優先し、未設定なら既存の保存用token（LINEはPUSH_SUBSCRIPTIONS_GITHUB_TOKEN、DiscordはGITHUB_DATA_TOKEN）を再利用する。api.github.comへの要求だけで使い、公開rawファイルには送らない。認証未設定時はGitHubの匿名API制限の対象となる。LINEの--liveはassetsだけをcommitへ固定し、event/別称repoは従来どおりmainの条件付き取得を使う。

## 確認方法

`cargo test --locked --release -p kbc-core asset_platform`は補完元の選択・パス制約・破損索引・期限超過時のstale拒否を確認する。`KBC_ASSET_TEST_REF=<40文字commit>`を指定し、末尾へ`-- --include-ignored --nocapture`を付けると公開データのキャラ名、UnitBuy、敵名、マップ名、イベント名、ミッション名、ガチャ対応表、PNGを取得する。LINE/Discordへのログインや投稿は行わない。

LINEの`node scripts/search-snapshot.cjs <検証先> --live`でut/tut/stの更新と検証を確認できる。providerの原本や公開中のデータは変更しない。Botの本番配送・多数OC/サーバーの負荷は、この公開データ取得の検証とは分けて扱う。

## 2026-10-22の確認結果

- 両Botで補完元・破損・期限超過のテストと、上記commitの公開ファイル取得を実行し、それぞれ2件成功。
- LINEの検索索引を新配置から生成し、ut 882件、tut 792件、st 7,748件を確認。生成物は検証用ディレクトリだけに保存した。
- LINEはTypeScript型検査・Node構文検査・Rust Clippy（all-targets、警告をエラー扱い）に成功。
- DiscordはRust checkに成功。通常の厳格Clippyは既存のtoo_many_arguments / field_reassign_with_default / collapsible_if / chunks_exact_to_as_chunksで失敗するため、この4種だけをCLIで許可して再実行し成功。無関係な実装は変更していない。
- LINE・Discordの本番接続、配送、配備は未実施。
