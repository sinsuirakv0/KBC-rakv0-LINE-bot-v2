# ログ変換Workflow

2026-10-03。旧BOTの実装を読み、機密データのローカルコピーで変換を試験した。公開repoには本文・名前・実MIDを置かない。

migrate.pyはPython標準ライブラリだけを使う。旧ログを一度走査し、SQLite一時索引でstream・時刻・IDをまとめる。encode_message / decode_messageはフォルダcontextと短い配列の可逆変換。metadataの同じMID・同じ時刻はbitmaskで復元する。未知のrecord属性はextraへ残す。同一記録だけを重複排除し、同じIDで情報が異なる記録は保持する。Migration.writeは4MiB目安で分割し、gzip保存後に全行を再展開照合する。

旧profilesとmanifestのbackfill・member集約も圧縮保管する。旧名前履歴のfirstSeenAt / lastSeenAt / countは観測範囲で、正確な改名時刻を捏造しない。所属不明はunmappedへ保管する。正常なJSONの後ろに破損断片があるファイルは、読めた記録を変換し、原ファイル全体をquarantineへ保存する。破損断片の修復完了とは扱わない。先頭から読めないファイルは変換を停止する。

ローカル試験: 入力408,330,210byte、出力13,557,829byte、payload107ファイル・89stream。発言424,492件、参加退出等4,775件、名前観測1,750件、profile2,136件。重複161,951件。破損原ファイル1件を保管。発言の復元内容を全件照合し、保存したgzipの全行も照合した。当時のsnapshotでの値で、最新Workflow結果は別途確認する。

migrate.workflow.ymlは非公開データrepoの.github/workflows/migrate-logs-v2.ymlへ置くtemplate。CODE_REF_PLACEHOLDERを公開Botの確認済みcommitへ置き換える。公式公開版checkout / upload-artifact v7.0.1を確認して採用。dataはmain、変換コードはcommit固定。apply=falseで試験し、apply=trueで旧状態のlogs-backup/<run-id> branchを保存してから新形式をmainへ反映する。Git履歴のpurgeは行わない。整理対象は旧ログ3パスのみ。認証・設定・Runtime・他機能は変更しない。

初回切替中は旧BOTと新形式への書込を止める。変換済みreportがあれば再実行を止める。Coreの毎分退避がmainへcommitしても、ログ変更だけをrebaseして再試行する。Git push失敗では旧フォルダの遠隔削除は起きない。

[保存形式](../../../docs/decisions/OC_LOG_STORAGE_V2.md)、[Core受付](../../../crates/kbc-core/docs/LOGS.md)。
