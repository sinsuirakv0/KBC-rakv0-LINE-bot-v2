# OC処分の応答と結果確定

2026-10-17。`oc kick`、即抜けのBANNED、危険語のKICK_OUT、test kickが使う既存membership要求の修正。Protocol v23と採用LINEJS 3.4.2を維持する。

## 問題と判断

更新応答へ通常のプロフィール取得と同じ`memberDto`を必須適用していた。更新後のmembershipStateが返っていても、squareMid・revision等が省略されていればInvalidSquareMemberResponseとなり、処分済みでもunknownになり得た。既存smokeは常に完全なプロフィールを返していたため、この応答形を確認していなかった。利用者の追加報告では、退会済みの相手への再参加禁止でILLEGAL_ARGUMENTが返り、Botに権限があり、実際には禁止できている可能性があるとのこと。実APIの応答と操作後の状態は未取得であり、部分応答を本番の唯一の原因とは断定しない。

旧LINEのocSquareBan.ts、採用SDKのbase/service/square/mod.tsとThrift定義を確認した。MEMBERSHIP_STATEは属性5、KICK_OUT=5 / BANNED=6。UpdateSquareMemberResponseは同じ単一更新要求の返値で、プロフィール全項目の存在を処分成功の条件にする必要はない。部分応答を受理するが、本文にない名前・revision・所属は作らない。

## 関数と分岐

| 関数 | 動作 |
| --- | --- |
| SquareDirectory.execute(membership) | 入力state・revisionを検査し、updatedAttrs=[5]で1回更新。返却MID・OCが存在すれば要求との一致を検査。返却membershipStateが要求と一致すれば成功。完全なmemberDtoを作れない場合はresult.memberを省略する |
| 同関数の確認読み取り | 更新応答に状態がない、または要求状態と異なる場合だけgetSquareMemberを1回。現在MID・OC・状態の一致を確認できた場合に成功。新しい処分要求は送らない |
| 同関数のILLEGAL_ARGUMENT確認 | 構造化codeがILLEGAL_ARGUMENT / 400の場合もgetSquareMemberを1回。対象MID・OC・状態の一致で成功、状態が異なれば元の拒否でfailed、読み取り失敗・所属不一致はunknown。要求によって変化したか、既に希望状態だったかは推測しない |
| squareRequestRejected | 構造化SquareException.data.errorCodeが明確な拒否かを共通APIモジュールで判定。文字列名と数値表現を扱う |
| deliverAction | membershipの更新要求自体が明確に拒否された場合はfailed。通信開始後のその他の例外はunknown。既存送信境界・全API共通枠・30秒期限を維持 |
| commands::mutation / test::mutated | 確定結果・codeを既存履歴と返信へ反映。処分前に取得済みのJob内の対象名を使用し、部分応答のために架空のプロフィールを作らない |

通常の処分結果は`commands.api_detail`を通し、OK以外のcodeを`API: <code>`として表示する。`oc::complete`の処分前の対象取得・OC情報取得失敗も、既存mod.complete_01 / 02のcode差し込みへ渡す。未処分の取得失敗と、更新拒否と、更新後の確認失敗を返信から区別できる。ユーザーから「結果不明だけでAPI見出しがない」と追加報告があったため、従来の` / code`だけの表示と、取得理由を捨てる表示を改めた。認証値・生の例外本文は出さない。

明確な拒否はSDKのSquareErrorCodeに従うILLEGAL_ARGUMENT=400、AUTHENTICATION_FAILURE=401、FORBIDDEN=403、NOT_FOUND=404、REVISION_MISMATCH=409、PRECONDITION_FAILED=410、NOT_IMPLEMENTED=501。その他の数値・内部エラー・通信切断・5xx・再試行指示を推測でfailedへ変えない。エラー文面に含まれる文字列だけでは判定しない。

確認読み取りが失敗した場合は`MembershipVerificationFailed_<code>`へ包み、元の構造化errorCodeを更新要求の拒否として伝播させない。例えば更新応答が空、またはILLEGAL_ARGUMENT→確認GETがNOT_FOUNDならunknown、更新要求自体がFORBIDDENならfailed。この区別により、処分後にプロフィール取得が拒否されても「処分されなかった」と誤確定しない。

unknownは従来どおり自動再実行しない。更新が成功応答だったのに確認状態が異なる場合もunknown。完全な更新応答・状態だけの部分応答には追加GETを行わず、情報不足の更新応答とILLEGAL_ARGUMENTの場合だけGETを最大1回行う。新しいTimer・Queue・永続Table・全体の並列枠は追加しない。

旧版の運用資料には、KICK_OUTを先に送ると後続BANNEDがILLEGAL_ARGUMENTになったとの記録がある。今回もBANNEDへの直接更新を維持し、KICK_OUT経由のfallback・盲目的なrevision更新再試行は追加しない。

## 検証と範囲

既存smoke:ocへ、状態だけの応答で追加GETなし・空応答から1回GETで成功・FORBIDDENと数値409のfailed・ILLEGAL_ARGUMENTと数値400後にLEFTならfailed / BANNEDなら成功・確認GETのNOT_FOUNDがunknownになること・処分要求の非再送を追加した。処分前の対象取得が失敗した場合もAPI code付きで未処分と表示し、更新要求は送らない。既存の通信切断→unknown・再起動でも非再送の検証も維持する。実LINEの処分や本番配備は別途確認する。

LINEJSのnpm公開版は同日3.5.0を再確認。SDK内部受信・認証の更新を混ぜず、今回の処分判定は採用済み3.4.2で修正する。依存更新は通信・受信境界の互換性を別作業で確認する。

検証結果: TypeScript check/build、Native release build、Core/Protocol Clippy、smoke:oc / smoke:messages / smokeが成功。上記の部分応答・拒否後の確認・確認失敗は採用SDKのメソッドを置き換えたオフライン検証であり、実LINEのILLEGAL_ARGUMENT発生理由を確定した結果ではない。
