use super::test_reply::take_word;
use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct Plan {
    operation: String,
    chat: String,
    members: Vec<String>,
    message: String,
    text: String,
    apply: bool,
    context: Option<OcContext>,
}

pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS oc_test_squares(square TEXT PRIMARY KEY);")?;
    Ok(())
}

fn mid(value: &str, prefix: char) -> bool {
    value.starts_with(prefix)
        && (9..=64).contains(&value.len())
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn permitted(db: &Connection, square: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM oc_test_squares WHERE square=?1)",
        [square],
        |row| row.get(0),
    )?)
}

pub fn parse(text: &str) -> Option<Input> {
    let mut input = text;
    let command = take_word(&mut input);
    if !command.eq_ignore_ascii_case("!test") && !command.eq_ignore_ascii_case("o.test") {
        return None;
    }
    let operation = take_word(&mut input).to_ascii_lowercase();
    if !matches!(
        operation.as_str(),
        "allow" | "mention" | "delete" | "kick" | "deputy" | "admin"
    ) || input
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("help"))
    {
        return None;
    }
    Some(Input {
        name: "test".into(),
        args: vec![operation],
        body: input.into(),
    })
}

fn allow(tx: &Transaction<'_>, job: &Job, now: i64) -> Result<()> {
    let mut args = job.input.body.split_whitespace().collect::<Vec<_>>();
    let remove = args.first() == Some(&"remove");
    if remove || args.first() == Some(&"add") {
        args.remove(0);
    } else if args.as_slice() == ["list"] {
        args.clear();
    }
    if args.is_empty() {
        let mut statement = tx.prepare("SELECT square FROM oc_test_squares ORDER BY square")?;
        let squares = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        return reply(
            tx,
            job,
            if squares.is_empty() {
                "検証OCは未登録です。!test allow <sMID> <sMID> ... で登録できます。".into()
            } else {
                format!(
                    "検証を許可したOC（{}件）\n{}",
                    squares.len(),
                    squares.join("\n")
                )
            },
            now,
        );
    }
    if args.len() > 16 || args.iter().any(|value| !mid(value, 's')) {
        return reply(
            tx,
            job,
            "検証OCはsから始まるMIDで、一度に16件まで指定してください。!test allow remove <sMID> ... で解除できます。",
            now,
        );
    }
    let mut squares = tx
        .prepare("SELECT square FROM oc_test_squares")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?;
    for value in &args {
        if remove {
            squares.remove(*value);
        } else {
            squares.insert((*value).into());
        }
    }
    if squares.len() > 64 {
        return reply(
            tx,
            job,
            "検証OCの登録上限は64件です。不要な登録をremoveで解除してください。",
            now,
        );
    }
    for value in args.iter().collect::<std::collections::BTreeSet<_>>() {
        if remove {
            tx.execute("DELETE FROM oc_test_squares WHERE square=?1", [value])?;
        } else {
            tx.execute("INSERT OR IGNORE INTO oc_test_squares VALUES(?1)", [value])?;
        }
    }
    reply(
        tx,
        job,
        format!(
            "検証OCを{}しました（現在{}件）。\n{}",
            if remove { "解除" } else { "登録" },
            squares.len(),
            args.join("\n")
        ),
        now,
    )
}

fn plan(job: &Job) -> Option<Plan> {
    let mut input = job.input.body.as_str();
    let mut operation = job.input.args[0].clone();
    if operation == "deputy" {
        operation = match take_word(&mut input) {
            "on" => "deputy-on",
            "off" => "deputy-off",
            _ => return None,
        }
        .into();
    }
    let target = take_word(&mut input);
    let deletion = operation == "delete";
    if if deletion {
        target.is_empty() || target.len() > 32 || !target.bytes().all(|byte| byte.is_ascii_digit())
    } else {
        !mid(target, 'p')
    } {
        return None;
    }
    let mut result = Plan {
        operation,
        chat: identity(&job.event).1.into(),
        members: if deletion {
            vec![]
        } else {
            vec![target.into()]
        },
        message: if deletion {
            target.into()
        } else {
            String::new()
        },
        text: "メンション通知テスト".into(),
        apply: false,
        context: None,
    };
    let mut chat_set = false;
    while !input.is_empty() {
        match take_word(&mut input) {
            "--target-chat" if !chat_set => {
                let chat = take_word(&mut input);
                if !mid(chat, 'm') {
                    return None;
                }
                result.chat = chat.into();
                chat_set = true;
            }
            "--from" if result.operation == "admin" && result.members.len() == 1 => {
                let member = take_word(&mut input);
                if !mid(member, 'p') || member == target {
                    return None;
                }
                result.members.push(member.into());
            }
            "--apply" if !result.apply => result.apply = true,
            "--" if result.operation == "mention" => {
                if input.trim().is_empty() {
                    return None;
                }
                result.text = input.into();
                break;
            }
            _ => return None,
        }
    }
    Some(result)
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    if bot_rank(runtime, job) < 2 {
        return reply(
            tx,
            job,
            "このテストと検証OCの登録はBOT管理者専用です。",
            now,
        );
    }
    if job.input.args[0] == "allow" {
        return allow(tx, job, now);
    }
    if !permitted(
        tx,
        &job.context.as_ref().ok_or("MissingOcContext")?.square_id,
    )? {
        return reply(
            tx,
            job,
            "実行OCが検証対象に未登録です。!test allow <sMID> ... で管理下の検証OCを登録してください。",
            now,
        );
    }
    let Some(plan) = plan(job) else {
        return reply(
            tx,
            job,
            "テスト引数を確認してください。使い方: !test help",
            now,
        );
    };
    let request_value = OcRequest::Inspect {
        chat_id: plan.chat.clone(),
        member_ids: plan.members.clone(),
    };
    job.test = Some(plan);
    request(tx, job, request_value, Phase::TestInspect, now)
}

fn role(value: &str) -> &str {
    match value {
        "1" => "ADMIN",
        "2" => "CO_ADMIN",
        "10" => "MEMBER",
        _ => value,
    }
}

pub fn inspected(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    if !matches!(result.status, DeliveryStatus::Sent) {
        return reply(
            tx,
            job,
            format!(
                "検証対象の情報を取得できませんでした。操作は未実行です。API: {}",
                result.code
            ),
            now,
        );
    }
    if bot_rank(runtime, job) < 2 || identity(&job.event).4 < now - 60000 {
        return reply(
            tx,
            job,
            "テストの権限または受付期限を確認できませんでした。未実行です。",
            now,
        );
    }
    let value = result.oc_result.as_ref().ok_or("MissingOcResult")?;
    let context = value.context.as_ref().ok_or("MissingTestContext")?;
    let mut test = job.test.take().ok_or("MissingTestPlan")?;
    if !permitted(tx, &context.square_id)?
        || !permitted(
            tx,
            &job.context.as_ref().ok_or("MissingOcContext")?.square_id,
        )?
    {
        return reply(
            tx,
            job,
            "対象OCが検証対象に未登録です。対象トークが所属するOCのsMIDを!test allowで登録してください。操作は未実行です。",
            now,
        );
    }
    if context.actor.member_id != context.bot_member_id
        || context.actor.square_id != context.square_id
        || !matches!(context.actor.state.as_str(), "JOINED" | "2")
        || value.members.len() != test.members.len()
        || test.members.iter().any(|id| {
            !value.members.iter().any(|member| {
                member.member_id == *id
                    && member.square_id == context.square_id
                    && matches!(member.state.as_str(), "JOINED" | "2")
            })
        })
    {
        return reply(
            tx,
            job,
            "対象OC・メンバー・Botの参加状態が一致しません。未実行です。",
            now,
        );
    }
    let target = value.members.first();
    let api = match test.operation.as_str() {
        "mention" => {
            let member = target.ok_or("MissingTestMember")?;
            let label = format!(
                "@{}",
                if member.name.is_empty() {
                    "メンバー"
                } else {
                    &member.name
                }
            );
            let text = format!("{label}\n{}", test.text);
            if text.encode_utf16().count() > 1500 {
                return reply(
                    tx,
                    job,
                    "メンションを含む本文は1,500 UTF-16単位までです。",
                    now,
                );
            }
            OcRequest::Post {
                chat_id: test.chat.clone(),
                text,
                mention: MessageMention {
                    member_id: member.member_id.clone(),
                    start: 0,
                    end: label.encode_utf16().count() as u32,
                },
            }
        }
        "delete" => OcRequest::Delete {
            chat_id: test.chat.clone(),
            message_id: test.message.clone(),
        },
        "kick" => {
            let member = target.ok_or("MissingTestMember")?;
            if member.member_id == context.bot_member_id || role(&member.role) != "MEMBER" {
                return reply(
                    tx,
                    job,
                    "強制退会テストは検証用の一般メンバーだけを指定してください。Bot・管理者・副官は対象外です。",
                    now,
                );
            }
            OcRequest::Membership {
                square_id: context.square_id.clone(),
                member_id: member.member_id.clone(),
                revision: member.revision.clone(),
                state: "KICK_OUT".into(),
            }
        }
        "deputy-on" | "deputy-off" => {
            let mut member = target.ok_or("MissingTestMember")?.clone();
            let expected = if test.operation == "deputy-on" {
                "MEMBER"
            } else {
                "CO_ADMIN"
            };
            if role(&member.role) != expected {
                return reply(
                    tx,
                    job,
                    "副官設定onは一般メンバー、offは副官を指定してください。",
                    now,
                );
            }
            member.role = if test.operation == "deputy-on" {
                "CO_ADMIN"
            } else {
                "MEMBER"
            }
            .into();
            OcRequest::Roles {
                square_id: context.square_id.clone(),
                members: vec![member],
            }
        }
        "admin" => {
            let mut member = target.ok_or("MissingTestMember")?.clone();
            let mut previous = value.members.get(1).unwrap_or(&context.actor).clone();
            if role(&member.role) != "CO_ADMIN"
                || role(&previous.role) != "ADMIN"
                || member.member_id == previous.member_id
            {
                return reply(
                    tx,
                    job,
                    "管理人移行は現在の副官を指定します。Botが対象OCの管理人でない場合は --from <現在の管理人pMID> も指定してください。",
                    now,
                );
            }
            member.role = "ADMIN".into();
            previous.role = "CO_ADMIN".into();
            OcRequest::Roles {
                square_id: context.square_id.clone(),
                members: vec![previous, member],
            }
        }
        _ => return Err("InvalidTestOperation".into()),
    };
    test.context = Some(context.clone());
    if !test.apply {
        let targets = value
            .members
            .iter()
            .map(|member| {
                format!(
                    "{} ({}) / {}",
                    member.name,
                    member.member_id,
                    role(&member.role)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return reply(
            tx,
            job,
            format!(
                "【テスト対象確認・未実行】\n操作: {}\nトーク: {} ({})\nOC: {}\n対象Bot: {} / {}\n{}{}\n実行する場合は同じ引数へ --apply を追加してください（本文指定の -- より前）。",
                test.operation,
                context.chat_name,
                test.chat,
                context.square_id,
                context.bot_member_id,
                role(&context.bot_role),
                targets,
                if test.operation == "delete" {
                    format!("メッセージID: {}", test.message)
                } else if test.operation == "mention" {
                    format!("\n本文: {}", test.text)
                } else if test.operation == "admin" {
                    "\n現在の管理人→副官、対象の副官→管理人".into()
                } else if test.operation == "kick" {
                    "\n強制退会（再参加可能）".into()
                } else {
                    format!(
                        "\n変更後: {}",
                        if test.operation == "deputy-on" {
                            "CO_ADMIN"
                        } else {
                            "MEMBER"
                        }
                    )
                }
            ),
            now,
        );
    }
    // 権限不足の負試験も許可OC内で1回だけ行う。判断は対象側のサーバーへ委ねる。
    job.operation = format!("test-{}", test.operation);
    job.test = Some(test);
    request(tx, job, api, Phase::Mutation, now)
}

pub fn mutated(
    tx: &Transaction<'_>,
    job: &Job,
    action: &CoreAction,
    result: &ActionResult,
    resolving: bool,
    now: i64,
) -> Result<()> {
    let CoreAction::OcApi { action_id, .. } = action else {
        return Err("MissingTestAction".into());
    };
    let test = job.test.as_ref().ok_or("MissingTestPlan")?;
    let context = test.context.as_ref().ok_or("MissingTestContext")?;
    let status = match result.status {
        DeliveryStatus::Sent => "成功",
        DeliveryStatus::Failed => "失敗",
        DeliveryStatus::Unknown => "結果不明",
    };
    let target = test
        .members
        .first()
        .map_or(test.message.as_str(), String::as_str);
    tx.execute("INSERT INTO oc_history(square,target,actor,operation,status,detail,at,action,target_name,actor_name,reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'',?9,?10)
        ON CONFLICT(action) WHERE action<>'' DO UPDATE SET status=excluded.status,detail=excluded.detail,at=excluded.at", params![context.square_id,target,identity(&job.event).3,job.operation,status,result.code,now,action_id,job.context.as_ref().ok_or("MissingOcContext")?.actor.name,format!("対象トーク: {} / Bot: {}",test.chat,context.bot_role)])?;
    tx.execute("DELETE FROM oc_history WHERE id IN (SELECT id FROM oc_history ORDER BY id DESC LIMIT -1 OFFSET 2048)", [])?;
    if !resolving {
        let message = result
            .oc_result
            .as_ref()
            .and_then(|value| value.message_id.as_deref())
            .map_or(String::new(), |id| format!("\n送信メッセージID: {id}"));
        reply(
            tx,
            job,
            format!(
                "【テスト結果】{}: {status}\n対象トーク: {}\nAPI: {}{message}{}",
                test.operation,
                test.chat,
                result.code,
                if matches!(result.status, DeliveryStatus::Unknown) {
                    "\n通信後の結果を確定できません。対象を確認してください。自動再実行はしません。"
                } else {
                    ""
                }
            ),
            now,
        )?;
    }
    Ok(())
}
