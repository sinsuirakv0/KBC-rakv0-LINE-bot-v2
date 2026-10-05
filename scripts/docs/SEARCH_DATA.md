# 公開検索データの更新ツール

2026-10-05。既存search-snapshot.cjsの変換を本番にも再利用する。DiscordのURL・変換をRustへ二重実装する負担を避け、公開資料の取得・前処理を短命の別プロセスへ隔離する。検索・番号の解釈・権限・Session・配送は引き続きRust Coreが所有する。

| 関数 | 働きと関係 |
| --- | --- |
| read | raw mainをCache-Control: no-cacheとETagで取得。304だけディスク本文を使用し、404は許可された代替資料へ進む。失敗でstaleを返さない |
| map | 素材資料を最大4並列に制限 |
| main | 通常モードはcommit固定。--liveはmainを取得し、全変換・上限検証後にvalidatedAtMs付き索引をrenameで公開 |
| SearchDataUpdater.refresh | Node子プロセス1件、90秒期限・中断・stderr末尾2KiBを管理。失敗はmetricsへ記録して受信を止めない |
| SearchDataUpdater.run | 開始から90秒周期。前回終了まで次を起動せず、AbortSignalで取得・待機を終了 |
| SearchCatalog::load | Coreの利用時だけ読込・正規化。mainはsearchDataLive=trueで確認時刻の欠落・未来・120秒以上を拒否 |
| sessions::apply | 最新revisionと保存済み候補を照合。更新後に以前の番号を別項目として選ばない |

既定pathはstorage/search/catalog.json。Dockerは実行用にこのcjsをscripts/へ含める。healthと毎分metricsのsearchData.refreshes / failures / lastSuccessAtで確認する。初回取得中・期限超過時の検索には設定可能なsearch.data_unavailable、データ変更後の候補にはsearch.data_changedを返す。

公開mainの取得を2回実行し、882 / 792 / 7,748件と同じentries revisionを確認した。コマンドSmokeで再起動なしの内容反映、時刻だけの更新で候補維持、実データ変更時の候補失効、120秒以上の拒否・ping継続を検証する。本環境の長時間CPU・応答時間と上流公開遅延は未測定。

検証済み: Native/TS build・型検査・Clippy、基盤/Command/OC/文面Smoke。HTTPを模擬した34件の304、取得失敗時に索引・確認時刻を変更しないことを確認した。実Updater.refreshから公開mainを取得し、Nativeのut/tut/stで最新の末尾IDを解決した。Windowsローカルの6入力で受付からローカルAction取得まで中央値174ms・最大234ms、RSS約60MiB（LINE通信なし・0.2CPU制限なし）。本番資源・配送待ちを含む応答時間の保証には使わない。
