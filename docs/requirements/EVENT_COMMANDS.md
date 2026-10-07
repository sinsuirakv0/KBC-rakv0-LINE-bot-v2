# イベント参照コマンド

2026-10-07。利用者指定の確定要件。

- Discord Bot v2のgatya・sale・itemをLINE Bot v2へ移植する。
- gatyaのレート表示に不足していた激レアを含める。
- saleの検索候補は、LINEの既存検索と同じ一覧へのリプライによる番号選択にする。
- 一覧・ID/名前検索・JSON/raw表示をRust Coreで扱い、既存の有限HTTP・準備・配送・Sessionを共用する。

[実装・入力・関数・上限](../../crates/kbc-core/src/event_data/docs/COMMANDS.md)。
