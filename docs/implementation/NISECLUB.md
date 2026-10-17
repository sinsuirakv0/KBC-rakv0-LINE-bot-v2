# niseclub

`!niseclub / o.niseclub` でクラブ編集ページのURLkey付きリンクを返す。引数ありはヘルプ。既存コマンド経路を使い、外部通信や独自キューは追加しない。秘密値が未設定・32バイト未満なら設定不足の案内を返す。

`crates/kbc-core/src/url_key.rs` の `generate_url_key(secret, now_ms)` がJSTの固定6時間枠のキーを生成する。`club_link(now_ms)` が環境変数 `URLKEY_SECRET` を読み本番URLへ付加する。Webと両botに同じ秘密値を設定して再起動する。秘密値をヘルプ・ログ・Gitへ保存しない。

仕様の正本は `kbc-web/docs/url-key.md`。`floor((now_ms+32400000)/21600000)` を小文字36進にし、`kbc-urlkey:v1:<stamp>` のHMAC-SHA256先頭12バイトをbase64url・パディングなしで符号化する。キーは `1.<stamp>.<signature>`、現在23文字。JST 0・6・12・18時で失効する。利用者別・発行時から6時間ではない。

既存Clock/イベント時刻からUnixミリ秒を渡す。Discordは組込みCommand登録、LINEはprepareのCommandPlan::Text経路。ヘルプはcontent/help/niseclub.txt。LINEの文面キーはmessages.schema.jsonとcontent/messages/commands.txtに登録する。

確認はkbc-coreのコンパイルと `url_key::tests::matches_web_vector`。固定値 `test-secret-for-kbc-urlkey-32-bytes` / `1792224000000` から `1.1s0u.QPHyWhTQYFauKay6` を生成し、Webと一致させる。実チャットへのテスト送信は行わない。

2026-10-17確認: release Nativeを生成し、実Runtimeへ `!niseclub` と `o.niseclub` を投入した。返信ActionのURLkeyをWebのverifyUrlKeyで検証して両方成功。LINE APIへの送信は行っていない。稼働環境への秘密値設定・再配備とは区別する。
