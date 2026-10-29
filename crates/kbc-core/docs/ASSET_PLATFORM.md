# Assets保存先変更への対応

2026-10-29更新。公開assetsのmain（調査時90a07a66f89d6bdc2c8efb5e2195f343cb558716、15.7.4）は、最新版の素材本体をjp/Integrated-dataへ統合する。キャラ名、敵・ステージ名、ガチャ対応表、イベント名、画像、モーションを両Botの共通HTTP境界で解決する。[公開側の保存契約](https://github.com/sinsuirakv0/KBC-rakv0-assets/blob/90a07a66f89d6bdc2c8efb5e2195f343cb558716/docs/integrated-data.md)。

## データ契約

currentのstatus=verified / schemaVersion=1 / sourceRegion=jp、datasetIdとbuildId、generatedPathを検査する。storageVersionで保存形式を選び、未知の値は拒否する。

| storageVersion | 生成物 | 素材本体 |
| --- | --- | --- |
| 未指定・null・1 | generated/jp/datasets/<buildId>/ | 採用元索引のfiles[logicalPath].path |
| 2 | generated/jp/datasets/<versionCode>/ | jp/Integrated-data/<logicalPath> |

形式2のversionCodeは正の整数。Local原本の許可範囲もjp/Local/<versionCode>/となり、形式1ではjp/Local/<datasetId>/を使う。共通でjp/server/、overrides/jp/assets/、overrides/jp/music/を許可する。索引全体を検査するため、新しく含まれる音源やassets/close@2x.pngの@を受け付ける。空のパス要素・親参照・クエリ等は引き続き拒否する。

同じGit commitからcurrent→manifest→indexes/assets.jsonを読み、manifestのSHA-256、索引のサイズ/SHA-256とheaderを照合する。キャラJSONは引き続きmanifestのsite/characters.json。形式2の名前CSV・画像・描画素材はIntegrated-dataから読み、採用元索引のサイズ/SHA-256で実byte列を照合する。索引にないファイル、統合物の欠落や不一致を成功として扱わない。BOMはhash計算前には除去せず、JSON/文字列解釈時だけ処理する。

metadata/jp/integrated.jsonの全件索引は既存の採用元索引と重複するため常駐・再取得しない。公開側の契約どおりlogical pathから統合URLを作り、manifestがhash固定した採用元索引で各本文を検証する。これにより追加の約10.7MBのHTTPと保持を避ける。統合物が最新版で上書きされても、URLを同じGit commitへ固定するので取得途中に別版へ切り替わらない。

## 共通関数と関係

| 関数 | 役割 |
| --- | --- |
| asset_platform::AssetPlatform.load | mainをGit commitへ固定し、保存形式を判定してcurrent・manifest・採用元索引を1本に集約して読む。固定SHAではref照会不要。保持は全体で1 snapshot、利用期限120秒。期限後の更新失敗でstaleを返さない |
| Dataset.resolve | character-index.jsonを生成物のsite/characters.jsonへ、それ以外を保存形式に応じて統合URLまたは採用元URLへ解決。索引にないファイルはNone |
| Proof.verify | 取得した元byte列とdescriptorのsize / sha256を比較する |
| AssetService / HttpService | 既存Client・Semaphore・timeoutでresolverを呼ぶ。Command固有のHTTP ClientやQueueは追加しない |
| RemoteAssetSource.pinned / AssetService.pinned | motionの取得開始時にmainを固定SHAへ変える。sprite・imgcut・model・animationを同じ版から取得する |
| scripts/asset-platform.cjs::openAssets | LINEの検索更新で同じ保存形式の判定・header/hash検査を行い、charactersとtext(logicalPath)をsearch-snapshot.cjsへ渡す |

既存のjp/sitedata形式の文字列は、呼出側のlogical path指定として残す。HTTP境界で新配置へ解決するため、新しいmainの旧実体へ直接HTTP要求しない。ut/tut/st、sale/gatya/skd、既存motion代行も同じ経路を使う。

移行前の固定commitに限り、currentが404ならそのcommitの旧jp/sitedataを使う。保存済みジョブと固定fixtureの互換処理であり、mainの失敗や破損索引を旧配置へ切り戻す処理ではない。

## 上限と更新

今回の採用元索引は54,338件・11,139,780 bytes。索引の取得だけ16MiB、件数131,072を上限にする。本文を同時に全件取得せず、保持するのはpath・サイズ・hashのmetadataだけ。通常ファイルの上限はLINE 4MiB / Discord既定8MiB、共通HTTP枠はLINE 2 / Discord既定4を維持する。

Discordの既存Commandごとのmetadata cacheとSession方針は維持する。resolver自身の利用期限は120秒で、添付本体をcacheしない。LINEの検索更新は短命Nodeプロセスがcommit・current・索引を固定して読み、最新性120秒の既存catalogへ変換する。buildIdもcatalog revisionへ含め、版変更時に古い番号候補を再解釈しない。fileの選択案内には準備時に解決した実体URLを保存する。

mainの固定にGitHub APIを使う。GITHUB_TOKENを優先し、未設定なら既存の保存用token（LINEはPUSH_SUBSCRIPTIONS_GITHUB_TOKEN、DiscordはGITHUB_DATA_TOKEN）を再利用する。api.github.comへの要求だけで使い、公開rawファイルには送らない。認証未設定時はGitHubの匿名API制限の対象となる。LINEの--liveはassetsだけをcommitへ固定し、event/別称repoは従来どおりmainの条件付き取得を使う。

## 確認方法

`cargo test --locked --release -p kbc-core asset_platform`は形式未指定・1・2、統合URL、音源・@入りファイル、未知の形式、パス制約、破損索引、期限超過時のstale拒否を既存1件のテストで確認する。`KBC_ASSET_TEST_REF=<40文字commit>`を指定し、末尾へ`-- --include-ignored --nocapture`を付けると、新形式の公開データからキャラ名、UnitBuy、敵名、マップ名、イベント名、ミッション名、ガチャ対応表、PNG、imgcut・model・animation、@2x画像を取得する。LINE/Discordへのログインや投稿は行わない。

LINEの`node scripts/search-snapshot.cjs <検証先> --live`でut/tut/stの更新と検証を確認できる。providerの原本や公開中データは変更しない。Botの本番配送・多数OC/サーバーの負荷は公開ファイル取得の検証とは分けて扱う。

## 2026-10-22の確認結果

- 両Botで補完元・破損・期限超過のテストと、上記commitの公開ファイル取得を実行し、それぞれ2件成功。
- LINEの検索索引を新配置から生成し、ut 882件、tut 792件、st 7,748件を確認。生成物は検証用ディレクトリだけに保存した。
- LINEはTypeScript型検査・Node構文検査・Rust Clippy（all-targets、警告をエラー扱い）に成功。
- DiscordはRust checkに成功。通常の厳格Clippyは既存のtoo_many_arguments / field_reassign_with_default / collapsible_if / chunks_exact_to_as_chunksで失敗するため、この4種だけをCLIで許可して再実行し成功。無関係な実装は変更していない。
- LINE・Discordの本番接続、配送、配備は未実施。

## 2026-10-29の確認結果

- 公開commit 90a07a66f89d6bdc2c8efb5e2195f343cb558716に固定して検証。Integrated-dataの台帳と採用元索引は全54,338件でlogical path・size・SHA-256・sourcePathが一致した。
- 両Botで既存の形式互換・破損・失効テストと公開ファイル取得テストを実行し、それぞれ2件成功。新しい統合先から名前CSV、PNG、imgcut・model・animation、@2x画像をhash照合付きで取得した。
- LINEの検索更新を実行し、ut 882件・tut 792件・st 7,748件を生成。catalog revisionは399223ed511e19040edc5a6f43c7e6c146e80d2bbc4c5ddc0fd57e2c3c132435。検証生成物は.tools内に保存した。
- LINEの厳格Clippy（all-targets）、Node構文検査、両BotのRust整形検査に成功。DiscordのClippyは前回と同じ既存警告4種をCLIで許可して成功。依存やProtocol、通常ファイルの容量・並列枠は変更していない。
- 本番接続・本番配送・配備は未実施。
