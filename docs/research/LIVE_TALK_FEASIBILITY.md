# ライブトークの実装候補と録音の調査

調査日: 2026-10-08（JST）。対象: LINEJS / linejs-types 3.4.2、Node.js 24.15.0、新Bot `72df834`、旧Bot `c6796e0`。
状態: SDK・旧コード・LINE公式仕様の調査と、認証なしのシリアライズ・合成音声の確認を実施。ライブトークのコマンド、実LINEの開始・参加・録音は未実装・未検証。

## 1. 結論と優先順

ライブトークを開く操作は `livetalk.acquireLiveTalk` が実装候補となる。開始・終了・タイトル変更・招待URLは既存の短時間API経路へ追加しやすい。APIが存在することと、このBotの認証端末・権限でLINEが受け付けることは別であり、少数の検証OCで確認する。

録音には `joinLiveTalk` の返す音声接続情報を使う別の接続処理が必要。SDKにはOpus・通信Transport・音声Sinkの部品があるが、LiveTalk専用の接続から録音保存までを組み立てた公開機能は、採用版の確認範囲では見つからない。一般通話・グループ通話の部品をそのままOCライブトークへ適用できるとは扱わない。

| 順序 | 対象 | 完了条件 |
| --- | --- | --- |
| 1 | 状態受信・開始・終了・名前・URL | Botに権限があるOCで開始と終了を確認。サブトーク、他端末からの開始、Bot再起動後の状態を照合 |
| 2 | スピーカー承認・共同ホスト・招待・ライブ内の退場 | 招待と承認の複数段階、OC権限とライブ内の役割、対象MIDのscopeを確認 |
| 3 | 音声接続と短時間録音 | 対応する音声protocol、接続維持・退出、実音声、時間・容量上限とCPU / Memoryを確認 |
| 後続 | 予約開始・終了、開始通知、音声再生・読み上げ | 既存の期限・配送基盤で動作し、通常の受信・返信を遅延させない |

これは実装順の提案であり、コマンド名・利用権限・録音の保存先はまだ確定していない。

## 2. SDKにある操作

現Botは `BaseClient` を使うため、入口は `client.livetalk`。高水準の `Client` を使うサンプルでの `client.base.livetalk` と混同しない。`SquareLiveTalkService` は `/SQLV1`、Protocol 4で要求を送る薄いRPCラッパーであり、イベント監視・音声接続・保存を自動で行わない。

表の「候補」はAPIと型の存在を確認した意味で、実LINEでの成功確認ではない。

| 機能候補 | LINEJSメソッド | 入力・結果と未確認点 |
| --- | --- | --- |
| 新しく開く | `acquireLiveTalk` | トークmMID、タイトル、`PUBLIC / PRIVATE`、`APPROVAL / ALL`。結果は `LiveTalk`。型の公開区分とアプリの外部参加設定の対応、Botがホストとなる挙動は実測で確定 |
| 終了 | `forceEndLiveTalk` | mMIDと `sessionId`。OC管理者とライブ内のホスト権限を区別して確認 |
| タイトル・発言申請の可否を変更 | `updateLiveTalkAttrs` | `updatedAttrs` と `LiveTalk`。列挙される属性は `TITLE` と `ALLOW_REQUEST_TO_SPEAK` のみ。取得したrevisionを使い、開始時の公開区分・speakerSettingも後から自由に変更できるとは仮定しない |
| Botが参加 | `joinLiveTalk` | mMID、sessionId、`wantToSpeak`、`claimAdult`。結果にtoken・接続先・protocol等。RPC成功だけでは音声を聞けることや参加維持を証明しない |
| 招待URL | `getLiveTalkInvitationUrl` | mMIDとsessionIdからURL取得 |
| 招待リンクの解決 | `findLiveTalkByInvitationTicket` | invitationTicketから `LiveTalk` 等を取得 |
| 状態・タイトル・人数等 | `getLiveTalkInfoForNonMember`、Squareイベント | APIはmMID・sessionId・speakersを要求。非メンバー用APIの公開条件は要検証。既存 `getSquareInfoByChatMid` は本トーク探索用で、現在のライブ状態の照会とは異なる |
| 発言者の表示情報 | `getLiveTalkSpeakersForNonMember` | speakersを指定して表示名・画像・役割等を取得。全参加者MIDを列挙するAPIとは扱わない |
| メンバーをライブへ招待 | `inviteToLiveTalk` | mMID・sessionId・invitees。通知の有無・対象MIDの種類と件数上限は未確認 |
| 発言を申請・取消 | `requestToSpeak`、`cancelToSpeak` | Bot自身の申請。申請一覧の取得・通知経路も必要 |
| 発言者を承認・拒否 | `acceptSpeakers`、`rejectSpeakers` | targetMidsを指定。発言者管理でありOCの参加承認ではない |
| 発言へ招待・承認・拒否 | `inviteToSpeak`、`acceptToSpeak`、`rejectToSpeak` | 招待後は `inviteRequestId` を使用。招待だけで相手の状態が確定するとは仮定しない |
| リスナーへ変更 | `requestToListen`、`inviteToListen`、`acceptToListen` | 自身の変更と相手への招待を分ける。承認にinviteRequestIdを使う |
| ホスト・共同ホスト等 | `inviteToChangeRole`、`acceptToChangeRole` | targetRoleは `HOST / CO_HOST / GUEST`。役割変更は招待と承認を分け、管理者譲渡のAPIとしては扱わない |
| ライブから退場 | `kickOutLiveTalkParticipants` | 単独参加者または非OCメンバーの一括対象という型がある。OC本体の強制退会とは別操作 |
| 詳細イベントを取得 | `fetchLiveTalkEvents` | mMID・sessionId・syncToken・limit。結果はevents・syncToken・hasMore。音声データは含まれない |
| 手動通報 | `reportLiveTalk`、`reportLiveTalkSpeaker` | API候補はあるが、今回の開始・録音の優先対象には含めない |

追加の限界:

- `RemoveLiveTalkSubscriptionRequest` と生成済みThrift関数はあるが、`SquareLiveTalkService` の公開メソッドには `removeLiveTalkSubscription` がない。購読解除と音声退出は同じ意味とは限らない。
- 専用の `leaveLiveTalk`、全参加者一覧、録音ファイル取得、サーバー録音開始・停止、ライブ内リアクション送信の公開ラッパーは今回の採用版の確認範囲では見つからない。
- `LiveTalk` にannouncementがあり変更イベントも定義されるが、更新用属性にannouncementはない。型にフィールドがあるだけでBotから編集可能としない。

## 3. 状態取得・イベントとIDの関係

`squareChatMid` はライブが属するトークのmMID、親OCはsMID、所属メンバーはpMID。`sessionId` は開催1回の識別子で、再開催時に前回の値を流用しない。音声の `memberSessionId` とメンバーpMIDも別物として扱う。

| 取得元 | 型にある情報 | 実装での扱い |
| --- | --- | --- |
| Square `NOTIFIED_UPDATE_LIVE_TALK`（51） | mMID・sessionId・liveTalkOnAir | 開催・終了状態の候補。重複と古い開催を照合 |
| Square `NOTIFICATION_LIVE_TALK`（52） | ライブトークの通知payload | 通知用途。全変更を受信できるとは仮定しない |
| Square `NOTIFIED_UPDATE_LIVE_TALK_INFO`（53） | mMID・LiveTalk・liveTalkOnAir | タイトル・ホスト・開始時刻・参加者数・revision等を状態として保存 |
| `fetchLiveTalkEvents` | タイトル・announcement・OCメンバーrole・発言申請可否・メンバー情報の変更 | 必要な開催だけを補完。イベントのmember roleの型は `SquareMemberRole` で、`LiveTalkRole` と同一視しない |
| `joinLiveTalk` | hostMemberMid・memberSessionId・token・proto・voip / orion / polaris接続情報・speaker | Adapter内の音声接続に使用。tokenやcommParam等を返信・ログ・Coreの一般データへ出さない |

現Botの `normalizeEvent / normalizeEvents` は51〜53をCoreEventへ変換せず、Receiverの種別件数とignored件数に入る。PUSH接続があることだけではライブ状態をBotが理解したことにならない。まずこの正規化とversion付きProtocolの追加が必要。

SDKのPUSH managerのsign-onは3（Square）と5 / 8（Talk sync）を分岐し、現Bot Receiverは3のみ購読する。`fetchLiveTalkEvents` の専用PUSHを自動購読する処理は確認できていない。Square側の開催イベントと、ライブ専用の詳細イベントの取得を区別する。補完の頻度は未決定で、全OCを固定間隔で巡回する方式は提案しない。

途中再起動ではSQLite / GitHubへ保存した状態が古い可能性がある。現在の開催を再確認してから操作し、情報不足を「開催なし」に読み替えない。`SquareChat / SquareChatStatus` の公開型にLiveTalkの現在状態はないため、既存のトーク照会だけで起動時の開催を復元できるとは保証しない。

## 4. 録音で必要なこと

音声の経路案は次のとおり。矢印の接続成功は未確認。

```text
joinLiveTalk → 開催専用の音声Transportを確立・維持
→ 音声packetを連続受信・復号・並び直し
→ 保存用の圧縮音声、またはOpusからPCMへの変換
→ 有限の一時ファイルへ逐次保存 → 停止時に完成確認
```

採用版には `PlanetTransport`（一般 / グループ通話）、`AndromedaTransport`、Opus codec、`AudioSink / streamSink`、`CallSession.receiveInto / received` がある。ただし `CallSession.start` は一般通話の `client.call.acquireRoute` を使い、`joinLiveTalk` の返値を受け付ける専用入口ではない。JoinLiveTalkResponseはCallRoute / GroupCallRouteとも構造が異なる。フィールド名が似ていることを根拠にキャストして録音可能とは扱わない。

`@evex/linejs/call` の実行時exportにも注意する。3.4.2では `CallSession` は型exportで、実行時の値はundefinedだった。`createCallClient` は高水準Client向けの `startSession` を提供するが、現BotのBaseClientへ置き換え不要で使えるものではない。音声機能のために受信全体を高水準Clientへ作り替えない。

### 0.2コア・512MBでの保存案

- 初回は1つの検証トーク・1接続・短時間で確認し、通常のメッセージ受信lag、pingの応答、CPUとMemoryを同時に測る。多OC同時録音の可否はその後に決める。
- SDKの `bufferSink` は全PCM frameを配列へ追加するため長時間録音へ使わない。48kHz・16bit・monoをそのまま保持すると約5.76MB/分、345.6MB/時になる。これは音声本体だけの計算値であり実測ではない。
- 保存は逐次書込とし、時間・byte・空き容量・接続数を上限で管理する。SDKのAndromeda / Planet内部にも受信配列へのpushがあり、その箇所には件数・byte制限がない。ファイルの背圧だけでSDK内部が有限になるとは扱わず、書込停滞時は録音の停止・接続終了を行う。
- 圧縮packetを保存できれば変換の負荷を減らせる可能性があるが、packetの形式、時刻、欠落、コンテナ、複数発言者が混合されるかを確認してから選ぶ。話者別録音は未保証。
- 接続失敗、終了、録音停止、Bot停止のすべてでTransport・codec・Timer・fileを回収する。停止は通常のOutbox渋滞で待たせず、取消を実行する。録音中の再起動からの無欠落復元は保証しない。
- 現本環境には永続ディスクがない。一時ファイルはコンテナ交換で消える。保存先・保存期間・容量を別途決め、大きい録音を既存の設定・テキストログ用GitHub同期へ自動で混ぜない。

LINE公式ヘルプは、ライブトークの内容を録音してOCや他のプラットフォームへ共有する行為を原則禁止と案内している。個人用保存が一律に許可されるとも、このページからは判断できない。録音の用途と扱いを決める際はこの条件を確認し、参加者へ録音を案内する設計にする。今回は技術的な調査だけで、実音声の録音・共有は行っていない。

## 5. コマンドと実装境界の提案

コマンド名は仮に `!live` / `o.live` とする。既存 `!push` とは分け、次の最小範囲から着手する案:

```text
!live status
!live start タイトル
!live end
!live name 新しいタイトル
!live url
```

発言者・共同ホスト管理や `record start / stop / status` は、対応するAPIの実測・音声接続確認後に追加する候補。開始時の公開区分・発言承認設定、利用者側の実行権限、対象トーク指定方法は実装前に確定する。初期検証はBot管理者のみで、Bot自身が対象OCで持つ実権限も確認する案とする。Bot管理者設定がLINE側の管理者権限の代わりになるとは扱わない。

| 配置 | 役割・既存処理との関係 |
| --- | --- |
| Rust OC機能 | Command解析、利用権限、mMID / sMIDとsessionIdの整合、設定、操作の結果・終了期限 |
| Protocol | 小さな開催状態DTO、照会・操作要求、結果。現行v19から必要時にversionを更新。音声本体・SDKオブジェクト・認証情報をJSONで往復させない |
| TS Adapter | `SquareDirectory`等の既存OC照会境界からRPC実行とDTO化。RPCは共通ApiSchedulerを使う |
| Receiver | 51〜53を正規化し、既存の永続受付・重複排除へ渡す。詳細取得は開催単位のcursorを別管理 |
| 音声接続処理（必要時のみ追加） | 接続と停止・保存の寿命を持つ有限の処理。長時間音声接続で短時間RPCの2枠を占有しない。通常RPC枠と別に接続数・通信量・CPU上限を測る |
| content | 全返信・案内・helpを既存のキーとtxtへ置く |

開始・終了・招待の通信後に結果が不明なら自動で再実行しない。状態照会・開催イベントで照合する。権限不足を認証切れに読み替えて全Botを再ログインさせない。

## 6. 今回の確認結果と次の小さな実験

認証・本番設定を読まず、LINE APIを呼ばずに次を確認した。Botのソース・依存・Protocolは変更していない。

1. npmの最新公開版と採用済み配布物は3.4.2、typesも3.4.2。依存更新は不要。
2. `SquareService_acquireLiveTalk_args` へ架空mMID・title・PRIVATE・APPROVALを渡し、生成済みserializerが公開区分2・speakerSetting1を持つ要求を生成した。LINEが受理したという意味ではない。
3. Node.js 24.15.0でcall moduleのimportが成功。PlanetTransport・opusCodecFactory・streamSinkは実行時の関数、CallSessionの実行時exportはundefined。
4. `opusCodecFactory` を用い、48kHz monoの合成無音960 sample（20ms）をencode / decodeできた。packetは57byte、復号は960 sample。codecをcloseして終了。LiveTalkの通信・実音声・リアルタイム性能の検証ではない。
5. 旧Botにはライブ開催・録音Commandがなく、LiveTalk APIの利用は主に `getSquareInfoByChatMid` による本トーク探索。旧資料の「緊急音声通知の可能性」は実装済み機能として移植しない。

次の実LINE実験は、参加済みでBotに管理権限がある検証OCのトーク1つで、開始→51 / 53観測→状態表示→名前変更→URL→終了の順に行う案。開始条件・必要権限・返るsessionId・イベント配信元・拒否codeを記録し、録音のための接続は別の短時間実験に分ける。開始は管理者・共同管理者に限られ、サブトークでもトーク単位で開始できることが現在の公式ヘルプに記載されている。人数等の開始条件は実機で確認する。

録音へ進む際の未確認点は、joinLiveTalkが受け付ける端末・claimAdultの意味、実際のprotoと必要なTransport、心拍・退出・接続解除、音声の復号・混合・欠落補完、コンテナからのUDP / TCP接続、CPU / Memory、保存先・保持条件。返されたtoken・接続パラメーターは資料へ貼らない。

## 7. 根拠と参照ファイル

外部の一次資料（2026-10-08確認）:

- [LINE公式：ライブトークの基本仕様](https://help.line.me/line?contentId=20024812&lang=ja)：開始・終了権限、サブトーク、ホスト管理、招待等。
- [LINE公式：録音と共有について](https://help.line.me/line/smartphone?contentId=20027033&lang=ja)：録音した内容の共有・ミラー配信等の案内。
- [LINEJSのcall公開資料（最新表示3.4.2）](https://jsr.io/@evex/linejs/doc/call)：音声Transport、codec、Sinkなどの公開型。
- [LINEJS開発元のLiveTalk service](https://github.com/evex-dev/linejs/blob/main/packages/linejs/base/service/livetalk/mod.ts)：API入口。mainは変化するため、今回の判定はローカルのlock済み3.4.2配布物と照合したもの。

ローカルの調査対象と役割:

| ファイル | 確認した関係 |
| --- | --- |
| `node_modules/@evex/linejs/base/service/livetalk/mod.ts` | RPCラッパー→生成済みThrift→request。開始・管理APIの一覧 |
| `node_modules/@jsr/evex__linejs-types/line_types.ts` | LiveTalk / JoinLiveTalkResponse、操作要求、イベントの型 |
| `node_modules/@evex/linejs/base/thrift/readwrite/struct.ts` | acquire / join / events等の要求serializer、公開wrapperのない購読解除 |
| `node_modules/@evex/linejs/base/push/connManager.ts` | SDK標準PUSHのservice分岐。LiveTalk詳細購読との違い |
| `node_modules/@evex/linejs/client/features/call/{mod,session,audio,opus}.ts` | session作成→一般通話route取得→Transport→codec / Sink。BaseClientやLiveTalkに直結しない点 |
| `node_modules/@evex/linejs/client/features/call/{andromeda,planet/transport}.ts` | 音声接続、内部受信配列、停止・受信の寿命 |
| [現Adapter](../../apps/line/docs/ADAPTER.md)、[OC機能](../../crates/kbc-core/src/oc/docs/OC.md) | RPC・正規化・既存Outboxへ接続する境界 |
| `apps/line/src/adapter/{events,receiver,square}.ts`、`apps/line/src/main.ts` | 未正規化のライブイベント、service3購読、BaseClient、本トーク探索の現状 |
| 旧Bot `src/commands/oc.ts`、`src/moderation/ocMainChat.ts`、`docs/research/LINEJS_OPENCHAT_RESEARCH.md` | 本トーク探索と旧調査。ライブ録音の実装と混同しない |

仕様・調査済み・実LINE確認済みを分ける方針は[文書運用](../engineering/DOCUMENTATION.md)に従う。
