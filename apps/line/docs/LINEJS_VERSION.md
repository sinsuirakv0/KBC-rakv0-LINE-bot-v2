# LINEJSの採用版

2026-10-18、npm registryの`@jsr/evex__linejs`の最新公開版3.5.0を確認し、package.json / lockのaliasと対応typesを3.4.2から3.5.0へ更新した。過去の実験資料のversionは当時のまま残す。

公開tarballと既存のrequest/mod.ts、service/auth/mod.ts、push/connManager.tsを照合した。requestCoreの引数・SDK自動refresh条件（MUST_REFRESH_V3_TOKEN）・refreshTokenのrotation保存は同じ。3.5.0のPUSH変更は既定の通知取得・noopの非同期失敗をcatchするものだが、Botは該当handlerを独自実装へ置き換えており、SDK更新だけで今回の認証失敗から復旧するとは扱わない。

requestCore / fetch / fetchPushの境界、service 3のsign-on、Thrift型、refreshToken更新をオフラインで確認する。依存の更新でProtocolは変更しない。npmのlockのintegrityで配布物を固定し、node_modulesの独自パッチは追加しない。

検証: TypeScript check/build、smoke（実SDK transport・Thrift・PUSH）、smoke:oc、smoke:skd、smoke:persistenceを3.5.0で実行。実LINEの認証更新・配備後の長時間接続は未検証。
