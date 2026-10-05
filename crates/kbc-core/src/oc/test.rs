use super::test_reply::take_word;
use super::*;
use crate::messages::message;

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
        "allow" | "mention" | "delete" | "kick" | "deputy" | "admin" | "sticker"
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

fn allow(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &Job,
    now: i64,
) -> Result<()> {
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
                message!(message_catalog, "test.allow_01").into()
            } else {
                message!(
                    message_catalog,
                    "test.allow_02",
                    arg0 = squares.len(),
                    arg1 = squares.join("\n")
                )
            },
            now,
        );
    }
    if args.len() > 16 || args.iter().any(|value| !mid(value, 's')) {
        return reply(tx, job, message!(message_catalog, "test.allow_03"), now);
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
        return reply(tx, job, message!(message_catalog, "test.allow_04"), now);
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
        message!(
            message_catalog,
            "test.allow_05",
            arg0 = if remove {
                message!(message_catalog, "common.removed")
            } else {
                message!(message_catalog, "common.registered")
            },
            arg1 = squares.len(),
            arg2 = args.join("\n")
        ),
        now,
    )
}

fn plan<'a>(
    message_catalog: &'a crate::messages::Messages,
    job: &Job,
) -> std::result::Result<Plan, &'a str> {
    let mut input = job.input.body.as_str();
    let mut operation = job.input.args[0].clone();
    if operation == "deputy" {
        operation = match take_word(&mut input) {
            "on" => "deputy-on",
            "off" => "deputy-off",
            _ => return Err(message!(message_catalog, "test.plan_01")),
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
        return Err(message!(message_catalog, "test.plan_02"));
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
        text: message!(message_catalog, "test.plan_03").into(),
        apply: false,
        context: None,
    };
    let mut chat_set = false;
    while !input.is_empty() {
        match take_word(&mut input) {
            "--target-chat" if !chat_set => {
                let chat = take_word(&mut input);
                if !mid(chat, 'm') {
                    return Err(message!(message_catalog, "test.plan_04"));
                }
                result.chat = chat.into();
                chat_set = true;
            }
            "--from" if result.operation == "admin" && result.members.len() == 1 => {
                let member = take_word(&mut input);
                if !mid(member, 'p') || member == target {
                    return Err(message!(message_catalog, "test.plan_05"));
                }
                result.members.push(member.into());
            }
            "--apply" if !result.apply => result.apply = true,
            "--" if result.operation == "mention" => {
                if input.trim().is_empty() {
                    return Err(message!(message_catalog, "test.plan_06"));
                }
                result.text = input.into();
                break;
            }
            _ => return Err(message!(message_catalog, "test.plan_07")),
        }
    }
    Ok(result)
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let message_catalog = &runtime.content.messages;
    if bot_rank(runtime, job) < 2 {
        return reply(
            tx,
            job,
            message!(message_catalog, "common.bot_admin_only"),
            now,
        );
    }
    if job.input.args[0] == "allow" {
        return allow(message_catalog, tx, job, now);
    }
    if job.input.args[0] == "sticker" {
        return sticker(message_catalog, tx, job, now);
    }
    let plan = match plan(message_catalog, job) {
        Ok(plan) => plan,
        Err(message) => {
            return reply(
                tx,
                job,
                message!(message_catalog, "test.execute_01", message = message),
                now,
            );
        }
    };
    let square = &job.context.as_ref().ok_or("MissingOcContext")?.square_id;
    if !permitted(tx, square)? {
        return reply(
            tx,
            job,
            message!(message_catalog, "test.execute_02", square = square),
            now,
        );
    }
    let request_value = OcRequest::Inspect {
        chat_id: plan.chat.clone(),
        member_ids: plan.members.clone(),
    };
    job.test = Some(plan);
    request(tx, job, request_value, Phase::TestInspect, now)
}

fn sticker(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &mut Job,
    now: i64,
) -> Result<()> {
    let usage = message!(message_catalog, "test.sticker_01");
    let mut input = job.input.body.as_str();
    let package_id = take_word(&mut input);
    let sticker_id = take_word(&mut input);
    let mut version = None;
    let mut option = None;
    while !input.is_empty() {
        match take_word(&mut input) {
            "--version" if version.is_none() => version = Some(take_word(&mut input)),
            "--option" if option.is_none() => option = Some(take_word(&mut input)),
            _ => return reply(tx, job, usage, now),
        }
    }
    let version = version.unwrap_or("1");
    if [package_id, sticker_id, version].iter().any(|value| {
        value.is_empty() || value.len() > 64 || !value.bytes().all(|byte| byte.is_ascii_digit())
    }) || option.is_some_and(|value| {
        value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    }) {
        return reply(
            tx,
            job,
            message!(message_catalog, "test.sticker_02", usage = usage),
            now,
        );
    }
    let api = OcRequest::Sticker {
        package_id: package_id.into(),
        sticker_id: sticker_id.into(),
        text: message!(message_catalog, "test.sticker_alt").into(),
        version: version.into(),
        option: option.map(String::from),
    };
    let chat = identity(&job.event).1.to_owned();
    // 送信テストは実行トークだけを使い、管理操作のallow登録を要求しない。
    job.test = Some(Plan {
        operation: "sticker".into(),
        chat,
        members: vec![],
        message: message!(
            message_catalog,
            "test.sticker_03",
            package_id = package_id,
            sticker_id = sticker_id,
            version = version
        ),
        text: String::new(),
        apply: true,
        context: job.context.clone(),
    });
    job.operation = "test-sticker".into();
    request(tx, job, api, Phase::Mutation, now)
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
    let message_catalog = &runtime.content.messages;
    if !matches!(result.status, DeliveryStatus::Sent) {
        return reply(
            tx,
            job,
            message!(message_catalog, "test.inspected_01", arg0 = result.code),
            now,
        );
    }
    if bot_rank(runtime, job) < 2 || identity(&job.event).4 < now - 60000 {
        return reply(tx, job, message!(message_catalog, "test.inspected_02"), now);
    }
    let value = result.oc_result.as_ref().ok_or("MissingOcResult")?;
    let context = value.context.as_ref().ok_or("MissingTestContext")?;
    let mut test = job.test.take().ok_or("MissingTestPlan")?;
    let square = &job.context.as_ref().ok_or("MissingOcContext")?.square_id;
    if !permitted(tx, square)? {
        return reply(
            tx,
            job,
            message!(message_catalog, "test.inspected_03", square = square),
            now,
        );
    }
    if !permitted(tx, &context.square_id)? {
        return reply(
            tx,
            job,
            message!(
                message_catalog,
                "test.inspected_04",
                arg0 = test.chat,
                arg1 = context.square_id,
                arg2 = context.square_id
            ),
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
        return reply(tx, job, message!(message_catalog, "test.inspected_05"), now);
    }
    let target = value.members.first();
    let api = match test.operation.as_str() {
        "mention" => {
            let member = target.ok_or("MissingTestMember")?;
            let label = format!(
                "@{}",
                if member.name.is_empty() {
                    message!(message_catalog, "common.member")
                } else {
                    &member.name
                }
            );
            let text = format!("{label}\n{}", test.text);
            if text.encode_utf16().count() > 1500 {
                return reply(tx, job, message!(message_catalog, "test.inspected_06"), now);
            }
            // 指定トークはメンバーの所属確認に使い、投稿は実行トークへ返す。
            OcRequest::Post {
                chat_id: identity(&job.event).1.into(),
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
                return reply(tx, job, message!(message_catalog, "test.inspected_07"), now);
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
                return reply(tx, job, message!(message_catalog, "test.inspected_08"), now);
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
                return reply(tx, job, message!(message_catalog, "test.inspected_09"), now);
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
            message!(
                message_catalog,
                "test.inspected_10",
                arg0 = test.operation,
                arg1 = context.chat_name,
                arg2 = test.chat,
                arg3 = context.square_id,
                arg4 = context.bot_member_id,
                arg5 = role(&context.bot_role),
                targets = targets,
                arg7 = if test.operation == "delete" {
                    message!(message_catalog, "test.inspected_11", arg0 = test.message)
                } else if test.operation == "mention" {
                    message!(
                        message_catalog,
                        "test.inspected_12",
                        arg0 = identity(&job.event).1,
                        arg1 = test.text
                    )
                } else if test.operation == "admin" {
                    message!(message_catalog, "test.inspected_13").into()
                } else if test.operation == "kick" {
                    message!(message_catalog, "test.inspected_14").into()
                } else {
                    message!(
                        message_catalog,
                        "test.inspected_15",
                        arg0 = if test.operation == "deputy-on" {
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
    message_catalog: &crate::messages::Messages,
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
        ON CONFLICT(action) WHERE action<>'' DO UPDATE SET status=excluded.status,detail=excluded.detail,at=excluded.at", params![context.square_id,target,identity(&job.event).3,job.operation,status,result.code,now,action_id,job.context.as_ref().ok_or("MissingOcContext")?.actor.name,message!(message_catalog, "test.mutated_01", arg0 = test.chat, arg1 = context.bot_role)])?;
    tx.execute("DELETE FROM oc_history WHERE id IN (SELECT id FROM oc_history ORDER BY id DESC LIMIT -1 OFFSET 2048)", [])?;
    if !resolving {
        let message = result
            .oc_result
            .as_ref()
            .and_then(|value| value.message_id.as_deref())
            .map_or(String::new(), |id| {
                message!(message_catalog, "test.mutated_02", id = id)
            });
        let ids = if test.operation == "mention" {
            message!(
                message_catalog,
                "test.mutated_03",
                arg0 = identity(&job.event).1
            )
        } else if test.operation == "sticker" {
            format!("\n{}", test.message)
        } else {
            String::new()
        };
        reply(
            tx,
            job,
            message!(
                message_catalog,
                "test.mutated_04",
                arg0 = test.operation,
                status = message_catalog.status(status),
                arg1 = if test.operation == "mention" {
                    message!(message_catalog, "test.mutated_05")
                } else {
                    message!(message_catalog, "test.mutated_06")
                },
                arg2 = test.chat,
                ids = ids,
                arg3 = result.code,
                message = message,
                arg4 = if matches!(result.status, DeliveryStatus::Unknown) {
                    message!(message_catalog, "test.mutated_07")
                } else {
                    ""
                }
            ),
            now,
        )?;
    }
    Ok(())
}
