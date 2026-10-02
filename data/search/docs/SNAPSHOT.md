# 同梱検索データ

2026-10-02に旧LINE Botの追跡済み `data/search` から取り込んだ公開ゲームデータ。データの最終commitは `351e249`。実メッセージ・認証・OCログを含まない。最新ゲーム版に追従したデータであるとは保証しない。

`catalog.json` はユニット871件、敵786件、マップ・ステージ7,502件、1,420,845 byte。名前・別名・ID・JDBリンクを持つ。別ファイルのJSONとCSVを一つへまとめ、元データの重複配置は避ける。

再取り込み:

```powershell
node scripts/import-search.cjs D:\KBC\KBC-rakv0-line-bot\data\search 'データの取得元・commit'
```

スクリプトはcharaname.json、enemyname.json、Map_Name.csv、StageNameのCSVだけを読む。空欄や `@ / ＠` を飛ばしても元の列番号をstage IDに使う。これは旧実装の空欄除去による列番号のずれを避ける変更。日本編等の数値IDも検索できるよう、表示IDとJDB用IDを分けて保持する。

更新したsnapshotを確認してcommit・配備する。定期更新・GitHub Actionsはまだ設けていない。常時ポーリングや検索時のデータ取得は行わない。全snapshotのbyte指紋が変わった場合、以前の行番号を保持する番号選択Sessionは起動時に失効する。

読み込み・検索の上限と関数は [Command実装](../../../crates/kbc-core/src/commands/docs/COMMANDS.md)。
