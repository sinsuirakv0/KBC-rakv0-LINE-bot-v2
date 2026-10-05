use super::*;
use crate::commands::pagination::{self, PAGE_SIZE};
use crate::messages::message;
use unicode_normalization::UnicodeNormalization;

pub fn parse(text: &str) -> Option<Input> {
    let (command, tail) = word(text.trim());
    if !command.eq_ignore_ascii_case("!oc") && !command.eq_ignore_ascii_case("o.oc") {
        return None;
    }
    let (name, body) = word(tail);
    let name = match name.to_ascii_lowercase().as_str() {
        "" | "help" => return None,
        "secret" | "managehelp" => "adminhelp",
        "joinmes" | "joinmsg" | "joinmessage" => "join",
        "leavemes" | "leavemsg" | "leavemessage" | "leftmes" | "leftmsg" | "leftmessage"
        | "exitmes" | "exitmsg" => "leave",
        "link" | "linkurl" | "adlink" => "url",
        "mediadel" | "mediaburst" => "media",
        _ => name,
    }
    .to_ascii_lowercase();
    let args = body
        .split_whitespace()
        .take(32)
        .map(str::to_owned)
        .collect();
    Some(Input {
        name,
        args,
        body: body.into(),
    })
}
fn word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], text[end..].trim_start())
}
fn status(message_catalog: &crate::messages::Messages, settings: &Settings) -> String {
    let flag = |enabled| if enabled { "ON" } else { "OFF" };
    message!(
        message_catalog,
        "commands.status_01",
        arg0 = flag(settings.url),
        arg1 = settings.rules.len(),
        arg2 = flag(settings.media),
        arg3 = flag(settings.left),
        arg4 = flag(settings.danger),
        arg5 = flag(settings.cohort),
        arg6 = flag(settings.report),
        arg7 = settings
            .mod_room
            .as_deref()
            .unwrap_or(message!(message_catalog, "common.not_set")),
        arg8 = settings
            .main
            .as_deref()
            .unwrap_or(message!(message_catalog, "common.not_set")),
        arg9 = settings.mutes.len()
    )
}
fn setup_menu(message_catalog: &crate::messages::Messages, settings: &Settings) -> String {
    message!(
        message_catalog,
        "commands.setup_menu_01",
        arg0 = status(message_catalog, settings)
    )
}
pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let message_catalog = &runtime.content.messages;
    let context = job.context.as_ref().ok_or("MissingOcContext")?.clone();
    let name = job.input.name.as_str();
    if name == "kicktest" {
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.execute_01"),
            now,
        );
    }
    if name == "status" {
        return reply(
            tx,
            job,
            status(message_catalog, &settings(tx, &context.square_id)?),
            now,
        );
    }
    if name == "authority" {
        return reply(
            tx,
            job,
            message!(
                message_catalog,
                "commands.execute_02",
                arg0 = context.actor.role,
                arg1 = bot_rank(tx, job)?,
                arg2 = context.bot_role,
                arg3 = context.authority
            ),
            now,
        );
    }
    let (bot, oc) = match name {
        "kick" => (1, 0),
        "mute" => (1, 3),
        "setup" | "session" | "modroom" | "main" | "join" | "leave" | "adminhelp" | "case" => {
            (2, 2)
        }
        _ => (1, 2),
    };
    if !allowed(tx, job, bot, oc)? {
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.execute_03"),
            now,
        );
    }
    if name == "adminhelp" {
        return reply(
            tx,
            job,
            runtime
                .content
                .internal_help("oc-admin")
                .unwrap_or_default(),
            now,
        );
    }
    if name == "session" {
        return session_reply(message_catalog, tx, job, now);
    }
    if name == "case" {
        return case_reply(message_catalog, tx, job, now);
    }
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM oc_settings WHERE square=?1)",
        [&context.square_id],
        |r| r.get(0),
    )?;
    if !existing
        && tx.query_row("SELECT count(*) FROM oc_settings", [], |r| {
            r.get::<_, i64>(0)
        })? >= 2048
    {
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.execute_04"),
            now,
        );
    }
    let mut value = settings(tx, &context.square_id)?;
    value.source_chat = Some(identity(&job.event).1.into());
    value.bot_member = Some(context.bot_member_id.clone());
    value
        .mutes
        .retain(|_, mute| mute.until.is_none_or(|until| until > now));
    let arg = job.input.args.first().map(String::as_str).unwrap_or("");
    match name {
        "setup" => {
            if arg == "status" {
                return reply(tx, job, status(message_catalog, &value), now);
            }
            return session(
                message_catalog,
                tx,
                job,
                Session::Setup,
                setup_menu(message_catalog, &value),
                now,
            );
        }
        "modroom" => match arg {
            "set" | "on" => value.mod_room = Some(identity(&job.event).1.into()),
            "off" | "del" | "remove" => value.mod_room = None,
            "test" => {
                if let Some(chat) = value.mod_room {
                    text_action(
                        tx,
                        &job.event,
                        "mod-test",
                        &chat,
                        message!(message_catalog, "commands.execute_05").into(),
                        TextDelivery::default(),
                        now,
                    )?;
                } else {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.execute_06"),
                        now,
                    );
                }
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.execute_07"),
                    now,
                );
            }
            _ => {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.modroom_usage"),
                    now,
                );
            }
        },
        "main" => {
            if matches!(arg, "set" | "here") {
                value.main = Some(identity(&job.event).1.into());
            } else if matches!(arg, "off" | "del") {
                value.main = None;
            } else {
                job.operation = "main".into();
                return request(
                    tx,
                    job,
                    OcRequest::Chats {
                        square_id: context.square_id,
                    },
                    Phase::Chats,
                    now,
                );
            }
        }
        "url" => match arg {
            "on" => value.url = true,
            "off" => value.url = false,
            "add" => {
                let raw = job.input.args.get(1).map(String::as_str).unwrap_or("");
                let scope = job.input.args.get(2).map(String::as_str).unwrap_or("exact");
                let Some(rule) = policy::UrlRule::new(raw, scope) else {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.execute_08"),
                        now,
                    );
                };
                if !value.rules.contains(&rule) {
                    if value.rules.len() >= 100 {
                        return reply(
                            tx,
                            job,
                            message!(message_catalog, "commands.execute_09"),
                            now,
                        );
                    }
                    value.rules.push(rule);
                }
            }
            "del" | "remove" => {
                let target = job.input.args.get(1).map(String::as_str).unwrap_or("");
                if target == "all" {
                    value.rules.clear();
                } else if let Some(index) = target
                    .parse::<usize>()
                    .ok()
                    .filter(|i| *i > 0 && *i <= value.rules.len())
                {
                    value.rules.remove(index - 1);
                } else {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.execute_10"),
                        now,
                    );
                }
            }
            "list" | "rules" => {
                let page = job
                    .input
                    .args
                    .get(1)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 20);
                let lines = value
                    .rules
                    .iter()
                    .enumerate()
                    .skip((page - 1) * 5)
                    .take(5)
                    .map(|(index, rule)| format!("{} {} {}", index + 1, rule.scope, rule.url))
                    .chain(std::iter::once(message!(
                        message_catalog,
                        "commands.execute_11",
                        page = page,
                        arg0 = value.rules.len().div_ceil(5).max(1)
                    )))
                    .collect::<Vec<_>>()
                    .join("\n");
                return reply(
                    tx,
                    job,
                    if lines.is_empty() {
                        message!(message_catalog, "commands.execute_12").into()
                    } else {
                        lines
                    },
                    now,
                );
            }
            _ => {
                return reply(
                    tx,
                    job,
                    message!(
                        message_catalog,
                        "commands.execute_13",
                        arg0 = value.url,
                        arg1 = value.rules.len()
                    ),
                    now,
                );
            }
        },
        "media" => match arg {
            "" | "on" => value.media = true,
            "off" | "del" => value.media = false,
            _ => {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.media_usage"),
                    now,
                );
            }
        },
        "watch" => {
            let enabled = match job.input.args.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.watch_usage"),
                        now,
                    );
                }
            };
            match arg {
                "early" => value.left = enabled,
                "danger" => value.danger = enabled,
                "cohort" => value.cohort = enabled,
                "report" => value.report = enabled,
                _ => {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.watch_usage"),
                        now,
                    );
                }
            }
        }
        "join" | "leave" => {
            if job.input.body.is_empty() {
                let notify = notifications(tx, identity(&job.event).1)?;
                return reply(
                    tx,
                    job,
                    match if name == "join" {
                        notify.join
                    } else {
                        notify.leave
                    } {
                        Some(template) => format!(
                            "{}\nmention={} / id={}",
                            template.text, template.mention, template.show_id
                        ),
                        None => message!(message_catalog, "commands.execute_14").into(),
                    },
                    now,
                );
            }
            let template = match template(&job.input.body) {
                Ok(value) => value,
                Err(_) => {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.execute_15"),
                        now,
                    );
                }
            };
            save_settings(tx, &context.square_id, &value)?;
            job.operation = name.into();
            if value.mod_room.as_deref() == Some(identity(&job.event).1) {
                return request(
                    tx,
                    job,
                    OcRequest::Chats {
                        square_id: context.square_id,
                    },
                    Phase::Chats,
                    now,
                );
            }
            if !save_notification(
                tx,
                &context.square_id,
                identity(&job.event).1,
                name,
                template,
            )? {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.execute_16"),
                    now,
                );
            }
            return reply(
                tx,
                job,
                message!(message_catalog, "commands.execute_17"),
                now,
            );
        }
        "mute" => {
            if arg == "list" {
                let page = job
                    .input
                    .args
                    .get(1)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 5);
                let text = value
                    .mutes
                    .iter()
                    .skip((page - 1) * 20)
                    .take(20)
                    .map(|(id, mute)| {
                        message!(
                            message_catalog,
                            "commands.execute_18",
                            arg0 = mute.name,
                            id = id,
                            arg1 = mute
                                .until
                                .map(|at| format!("{} JST", jst(message_catalog, at)))
                                .unwrap_or(message!(message_catalog, "common.unlimited").into())
                        )
                    })
                    .chain(std::iter::once(message!(
                        message_catalog,
                        "commands.execute_19",
                        page = page,
                        arg0 = value.mutes.len().div_ceil(20).max(1)
                    )))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                return reply(
                    tx,
                    job,
                    if text.is_empty() {
                        message!(message_catalog, "commands.execute_20").into()
                    } else {
                        text
                    },
                    now,
                );
            }
            job.operation = "mute".into();
            job.targets = targets(job);
            if job.targets.len() != 1 || job.input.args.is_empty() {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.execute_21"),
                    now,
                );
            }
            return next_target(message_catalog, tx, job, now);
        }
        "kick" => {
            if matches!(arg, "his" | "history") {
                return show_history(message_catalog, tx, job, now);
            }
            job.operation = "manual-ban".into();
            job.targets = targets(job);
            if job.targets.is_empty() || job.targets.len() > 8 {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.execute_22"),
                    now,
                );
            }
            return next_target(message_catalog, tx, job, now);
        }
        "history" => return show_history(message_catalog, tx, job, now),
        _ => {
            return reply(
                tx,
                job,
                message!(message_catalog, "commands.execute_23"),
                now,
            );
        }
    }
    save_settings(tx, &context.square_id, &value)?;
    reply(tx, job, status(message_catalog, &value), now)
}
fn template(input: &str) -> Result<Option<Template>> {
    let (first, tail) = word(input);
    if matches!(first, "off" | "del" | "remove") {
        return Ok(None);
    }
    let mut tail = if first == "set" { tail } else { input };
    let mut mention = false;
    let mut show_id = false;
    loop {
        let (flag, next) = word(tail);
        match flag {
            "--mention" | "mention" => mention = true,
            "--id" | "id" => show_id = true,
            _ => break,
        }
        tail = next;
    }
    if tail.is_empty() || tail.len() > 4096 || tail.encode_utf16().count() > 1300 {
        return Err("OcTemplateLimit".into());
    }
    Ok(Some(Template {
        text: tail.into(),
        mention,
        show_id,
    }))
}
fn save_notification(
    tx: &Transaction<'_>,
    square: &str,
    chat: &str,
    kind: &str,
    template: Option<Template>,
) -> Result<bool> {
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM oc_notifications WHERE chat=?1)",
        [chat],
        |r| r.get(0),
    )?;
    if !existing
        && tx.query_row("SELECT count(*) FROM oc_notifications", [], |r| {
            r.get::<_, i64>(0)
        })? >= 2048
    {
        return Ok(false);
    }
    let mut value = notifications(tx, chat)?;
    if kind == "join" {
        value.join = template
    } else {
        value.leave = template
    };
    tx.execute("INSERT INTO oc_notifications VALUES(?1,?2,?3) ON CONFLICT(chat) DO UPDATE SET square=excluded.square,payload=excluded.payload",params![chat,square,serde_json::to_string(&value)?])?;
    if tx.query_row("SELECT count(*) FROM oc_notifications", [], |r| {
        r.get::<_, i64>(0)
    })? > 2048
    {
        return Err("OcNotificationLimit".into());
    }
    Ok(true)
}
pub(super) fn targets(job: &Job) -> Vec<String> {
    let mut result = Vec::new();
    if let CoreEvent::MessageReceived { mentions, .. } = &job.event {
        for id in mentions.iter().flatten() {
            if member_id(id) && !result.contains(id) {
                result.push(id.clone());
            }
        }
    }
    for token in &job.input.args {
        let token = token
            .split_once(':')
            .map_or(token.as_str(), |(_, value)| value);
        if member_id(token) && !result.iter().any(|id| id == token) {
            result.push(token.into());
        }
    }
    result
}
fn member_id(text: &str) -> bool {
    text.len() >= 9
        && text.len() <= 64
        && text.starts_with('p')
        && text[1..].bytes().all(|c| c.is_ascii_hexdigit())
}
pub(super) fn reason(message_catalog: &crate::messages::Messages, job: &Job) -> String {
    match job.operation.as_str() {
        "danger-kick" => return message!(message_catalog, "commands.reason_01").into(),
        "left-ban" => return message!(message_catalog, "commands.reason_02").into(),
        "case-ban" => return message!(message_catalog, "commands.reason_03").into(),
        _ => {}
    }
    job.input
        .args
        .iter()
        .filter(|arg| {
            let value = arg.split_once(':').map_or(arg.as_str(), |(_, value)| value);
            !member_id(value)
                && !arg.starts_with('@')
                && !matches!(arg.to_ascii_lowercase().as_str(), "userid" | "mid")
        })
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}
pub fn next_target(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &mut Job,
    now: i64,
) -> Result<()> {
    if let Some(id) = job.targets.first().cloned() {
        request(
            tx,
            job,
            OcRequest::Member { member_id: id },
            Phase::Target,
            now,
        )
    } else {
        let text = if job.operation == "manual-ban" {
            message!(
                message_catalog,
                "commands.next_target_01",
                arg0 = job.results.join("\n"),
                arg1 = if reason(message_catalog, job).is_empty() {
                    message!(message_catalog, "common.not_specified").into()
                } else {
                    reason(message_catalog, job)
                }
            )
        } else {
            job.results.join("\n")
        };
        if let Some(context) = &job.context
            && let Some(room) = settings(tx, &context.square_id)?.mod_room
            && room != identity(&job.event).1
        {
            text_action(
                tx,
                &job.event,
                &format!("result-log-{}", job.step),
                &room,
                message!(
                    message_catalog,
                    "commands.next_target_02",
                    arg0 = if job.operation == "manual-ban" {
                        message!(message_catalog, "commands.next_target_03")
                    } else {
                        message!(message_catalog, "commands.next_target_04")
                    },
                    arg1 = identity(&job.event).1,
                    arg2 = context.actor.name,
                    arg3 = context.actor.member_id,
                    text = text
                ),
                TextDelivery::default(),
                now,
            )?;
        }
        reply(tx, job, text, now)
    }
}
pub fn target(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let message_catalog = &runtime.content.messages;
    let member = result
        .oc_result
        .as_ref()
        .and_then(|value| value.member.clone())
        .ok_or("MissingOcMember")?;
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    if member.square_id != context.square_id || job.targets.first() != Some(&member.member_id) {
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.target_01"),
            now,
        );
    }
    job.target_member = Some(member.clone());
    if job.operation == "mute" {
        let duration = job
            .input
            .args
            .iter()
            .rev()
            .find(|arg| {
                !member_id(arg.split_once(':').map_or(arg.as_str(), |(_, value)| value))
                    && !arg.starts_with('@')
            })
            .map(String::as_str)
            .unwrap_or("");
        let mut value = settings(tx, &context.square_id)?;
        value.source_chat = Some(identity(&job.event).1.into());
        value.bot_member = Some(context.bot_member_id.clone());
        if matches!(duration, "off" | "del" | "remove" | "解除") {
            value.mutes.remove(&member.member_id);
        } else if let Some(until) = policy::mute_until(duration, now) {
            if value.mutes.len() >= 100 && !value.mutes.contains_key(&member.member_id) {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.target_02"),
                    now,
                );
            }
            value.mutes.insert(
                member.member_id.clone(),
                Mute {
                    until,
                    name: member.name.clone(),
                    since: now,
                },
            );
        } else {
            return reply(
                tx,
                job,
                message!(message_catalog, "commands.target_03"),
                now,
            );
        }
        save_settings(tx, &context.square_id, &value)?;
        history(
            message_catalog,
            tx,
            job,
            &member.member_id,
            "saved",
            duration,
            now,
        )?;
        let period = value.mutes.get(&member.member_id).map(|mute| {
            mute.until.map_or(
                message!(message_catalog, "common.unlimited").into(),
                |until| {
                    message!(
                        message_catalog,
                        "commands.target_04",
                        arg0 = jst(message_catalog, until),
                        arg1 = policy::duration(message_catalog, until - now)
                    )
                },
            )
        });
        let text = period.as_ref().map_or(
            message!(message_catalog, "commands.target_05", arg0 = member.name),
            |period| {
                message!(
                    message_catalog,
                    "commands.target_06",
                    arg0 = member.name,
                    period = period
                )
            },
        );
        if let Some(room) = value.mod_room.filter(|room| room != identity(&job.event).1) {
            text_action(
                tx,
                &job.event,
                "mute-log",
                &room,
                message!(
                    message_catalog,
                    "commands.target_07",
                    arg0 = identity(&job.event).1,
                    arg1 = context.actor.name,
                    arg2 = context.actor.member_id,
                    arg3 = member.name,
                    arg4 = member.member_id,
                    text = text
                ),
                TextDelivery::default(),
                now,
            )?;
        }
        return reply(tx, job, text, now);
    }
    if member.member_id == context.bot_member_id
        || policy::role_rank(&member.role) != 1
        || crate::permissions::rank(
            tx,
            &member.square_id,
            identity(&job.event).1,
            &member.member_id,
        )? > 0
    {
        job.results.push(message!(
            message_catalog,
            "commands.target_08",
            arg0 = member.name,
            arg1 = member.member_id
        ));
        job.targets.remove(0);
        return next_target(message_catalog, tx, job, now);
    }
    if job.operation == "left-ban" && !matches!(member.state.as_str(), "LEFT" | "4") {
        return Ok(());
    }
    if job.operation == "left-check" {
        return moderation::confirm_left(message_catalog, tx, job, &member, now);
    }
    let state = if job.operation == "danger-kick" {
        "KICK_OUT"
    } else {
        "BANNED"
    };
    request(
        tx,
        job,
        OcRequest::Membership {
            square_id: member.square_id,
            member_id: member.member_id,
            revision: member.revision,
            state: state.into(),
        },
        Phase::Mutation,
        now,
    )
}
pub fn mutation(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let target = job.targets.first().cloned().ok_or("MissingOcTarget")?;
    let status = match result.status {
        DeliveryStatus::Sent => "成功",
        DeliveryStatus::Unknown => "結果不明（自動再試行しません）",
        DeliveryStatus::Failed => "失敗",
    };
    let label = message_catalog.status(status);
    history(
        message_catalog,
        tx,
        job,
        &target,
        status,
        &format!("{} / {}", result.code, job.input.body),
        now,
    )?;
    let name = job
        .target_member
        .as_ref()
        .filter(|member| member.member_id == target)
        .or_else(|| {
            job.context
                .as_ref()
                .map(|context| &context.actor)
                .filter(|member| member.member_id == target)
        })
        .map_or(message!(message_catalog, "common.member"), |member| {
            member.name.as_str()
        });
    job.results.push(message!(
        message_catalog,
        "commands.mutation_result",
        status = label,
        name = name,
        target = target,
        detail = if result.code == "OK" {
            String::new()
        } else {
            format!(" / {}", result.code)
        }
    ));
    if let Some(case) = &job.case_id {
        tx.execute(
            "UPDATE oc_cases SET state=?2 WHERE id=?1",
            params![
                case,
                if matches!(result.status, DeliveryStatus::Sent) {
                    "banned"
                } else {
                    "unknown"
                }
            ],
        )?;
    }
    job.targets.remove(0);
    if job.operation == "danger-kick" {
        let context = job.context.as_ref().ok_or("MissingOcContext")?;
        let joined: Option<i64> = tx
            .query_row(
                "SELECT joined FROM oc_members WHERE square=?1 AND member=?2",
                params![context.square_id, target],
                |row| row.get(0),
            )
            .optional()?;
        let (word, body) = if let CoreEvent::MessageReceived { text, .. } = &job.event {
            (
                policy::danger_word(text)
                    .unwrap_or(message!(message_catalog, "common.unavailable")),
                text.chars().take(300).collect::<String>(),
            )
        } else {
            (
                message!(message_catalog, "common.unavailable"),
                String::new(),
            )
        };
        let elapsed = joined.map_or(
            message!(message_catalog, "common.unavailable").into(),
            |joined| policy::duration(message_catalog, identity(&job.event).4 - joined),
        );
        moderation::case(
            message_catalog,
            tx,
            job,
            Some(&target),
            None,
            &message!(
                message_catalog,
                "commands.mutation_02",
                elapsed = elapsed,
                word = word,
                body = body,
                label = label,
                arg0 = job.results.join("\n")
            ),
            now,
        )?;
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.mutation_03", label = label),
            now,
        );
    }
    if job.operation == "left-ban" {
        moderation::case(
            message_catalog,
            tx,
            job,
            Some(&target),
            None,
            &message!(
                message_catalog,
                "commands.mutation_04",
                label = label,
                arg0 = job.results.join("\n")
            ),
            now,
        )?;
        let context = job.context.as_ref().ok_or("MissingOcContext")?;
        if matches!(result.status, DeliveryStatus::Sent)
            && let Some(main) = settings(tx, &context.square_id)?.main
        {
            text_action(
                tx,
                &job.event,
                "left-main-notice",
                &main,
                message!(
                    message_catalog,
                    "commands.mutation_05",
                    label = label,
                    arg0 = job.results.join("\n")
                ),
                TextDelivery::default(),
                now,
            )?;
        }
        return Ok(());
    }
    next_target(message_catalog, tx, job, now)
}
fn show_history(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &Job,
    now: i64,
) -> Result<()> {
    let square = &job.context.as_ref().ok_or("MissingOcContext")?.square_id;
    let kick_only = job.input.name == "kick";
    let mut query=tx.prepare("SELECT target,operation,status,detail,at,target_name,actor_name,reason,actor FROM oc_history WHERE square=?1 AND (?2=0 OR operation IN ('manual-ban','danger-kick','left-ban','case-ban')) ORDER BY id DESC LIMIT ?3")?;
    let rows = query
        .query_map(
            params![square, kick_only, if kick_only { 10 } else { 15 }],
            |r| {
                Ok(message!(
                    message_catalog,
                    "commands.show_history_01",
                    at = jst(message_catalog, r.get(4)?),
                    status = message_catalog.status(&r.get::<_, String>(2)?),
                    operation = match r.get::<_, String>(1)?.as_str() {
                        "danger-kick" => message!(message_catalog, "commands.show_history_02"),
                        "manual-ban" => message!(message_catalog, "commands.show_history_03"),
                        "left-ban" | "case-ban" =>
                            message!(message_catalog, "commands.show_history_04"),
                        "bot-name" => message!(message_catalog, "commands.show_history_05"),
                        operation => operation,
                    }
                    .to_owned(),
                    target_name = r.get::<_, String>(5)?,
                    target = r.get::<_, String>(0)?,
                    actor_name = r.get::<_, String>(6)?,
                    actor = r.get::<_, String>(8)?,
                    reason = r.get::<_, String>(7)?,
                    detail = r.get::<_, String>(3)?
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    reply(
        tx,
        job,
        if rows.is_empty() {
            message!(message_catalog, "commands.show_history_06").into()
        } else {
            format!(
                "{}\n\n{}",
                if kick_only {
                    message!(message_catalog, "commands.show_history_07")
                } else {
                    message!(message_catalog, "commands.show_history_08")
                },
                rows.join("\n\n")
            )
        },
        now,
    )
}
fn jst(message_catalog: &crate::messages::Messages, at: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at + 9 * 3600000)
        .map(|date| date.format("%Y/%m/%d %H:%M").to_string())
        .unwrap_or(message!(message_catalog, "common.unknown").into())
}
pub fn chats(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let chats = result
        .oc_result
        .as_ref()
        .ok_or("MissingOcChats")?
        .chats
        .clone();
    if chats.is_empty() || chats.len() > 64 {
        return reply(tx, job, message!(message_catalog, "commands.chats_01"), now);
    }
    let template = if job.operation == "main" {
        None
    } else {
        template(&job.input.body)?
    };
    let value = Session::Chats {
        chats,
        operation: job.operation.clone(),
        template,
        page: 0,
        page_size: PAGE_SIZE,
    };
    session(
        message_catalog,
        tx,
        job,
        value.clone(),
        chat_page(message_catalog, &value),
        now,
    )
}
fn chat_page(message_catalog: &crate::messages::Messages, value: &Session) -> String {
    let Session::Chats { chats, page, .. } = value else {
        return String::new();
    };
    let mut lines = vec![message!(message_catalog, "commands.chat_page_01").into()];
    let count = chats.len().div_ceil(PAGE_SIZE);
    lines.push(message!(
        message_catalog,
        "commands.chat_page_count",
        page = page + 1,
        pages = count
    ));
    for (index, chat) in chats
        .iter()
        .skip(page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .enumerate()
    {
        lines.push(format!(
            "{} {}{}",
            index + 1,
            if chat.is_main {
                message!(message_catalog, "commands.chat_page_02")
            } else {
                ""
            },
            chat.name
        ));
    }
    if count > 1 {
        lines.push(pagination::navigation(message_catalog, *page, count));
    }
    lines.push(message!(message_catalog, "navigation.finish").into());
    lines.join("\n")
}
fn session_reply(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &mut Job,
    now: i64,
) -> Result<()> {
    let id = job.input.args.first().ok_or("MissingOcSession")?;
    let stored:Option<(String,Option<String>)>=tx.query_row("SELECT payload,prompt FROM oc_sessions WHERE id=?1 AND chat=?2 AND owner=?3 AND expires>?4",params![id,identity(&job.event).1,identity(&job.event).3,now],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((payload, prompt)) = stored else {
        return Ok(());
    };
    let body = job
        .input
        .body
        .nfkc()
        .collect::<String>()
        .to_ascii_lowercase();
    if matches!(body.as_str(), "cancel" | "取消" | "終了") {
        tx.execute("DELETE FROM oc_sessions WHERE id=?1", [id])?;
        if let Some(prompt) = prompt {
            crate::enqueue_cleanup(
                tx,
                identity(&job.event).0,
                identity(&job.event).1,
                &prompt,
                now,
            )?;
        }
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.session_reply_01"),
            now,
        );
    }
    let value: Session = serde_json::from_str(&payload)?;
    let context = job.context.as_ref().ok_or("MissingOcContext")?.clone();
    match value {
        Session::Setup => {
            let (first, tail) = word(&body);
            let enabled = !matches!(first, "off" | "del");
            let numbers: Option<Vec<u8>> = if enabled { body.as_str() } else { tail }
                .split_whitespace()
                .map(|word| {
                    word.parse::<u8>()
                        .ok()
                        .filter(|number| (1..=11).contains(number))
                })
                .collect();
            let Some(numbers) = numbers.filter(|n| !n.is_empty() && n.len() <= 11) else {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.session_reply_02"),
                    now,
                );
            };
            if numbers.contains(&11) {
                if numbers.len() != 1 {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.session_reply_03"),
                        now,
                    );
                }
                job.operation = "main".into();
                return request(
                    tx,
                    job,
                    OcRequest::Chats {
                        square_id: context.square_id,
                    },
                    Phase::Chats,
                    now,
                );
            }
            let mut settings = settings(tx, &context.square_id)?;
            settings.source_chat = Some(identity(&job.event).1.into());
            settings.bot_member = Some(context.bot_member_id.clone());
            for number in numbers {
                match number {
                    1 => settings.url = enabled,
                    2 => settings.media = enabled,
                    3 => settings.mod_room = enabled.then(|| identity(&job.event).1.into()),
                    4 => settings.left = enabled,
                    5 => settings.danger = enabled,
                    6 => settings.cohort = enabled,
                    7 => {
                        settings.url = enabled;
                        settings.media = enabled;
                        settings.left = enabled;
                        settings.danger = enabled;
                        settings.cohort = enabled;
                        settings.mod_room = enabled.then(|| identity(&job.event).1.into())
                    }
                    8 => {
                        settings.url = false;
                        settings.media = false;
                        settings.left = false;
                        settings.danger = false;
                        settings.cohort = false;
                        settings.report = false
                    }
                    9 => {}
                    10 => settings.report = enabled,
                    _ => {}
                }
            }
            save_settings(tx, &context.square_id, &settings)?;
            session(
                message_catalog,
                tx,
                job,
                Session::Setup,
                setup_menu(message_catalog, &settings),
                now,
            )
        }
        Session::Chats {
            chats,
            operation,
            template,
            mut page,
            page_size,
        } => {
            if page_size != PAGE_SIZE {
                tx.execute("DELETE FROM oc_sessions WHERE id=?1", [id])?;
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.chat_page_expired"),
                    now,
                );
            }
            let number = match pagination::parse(&job.input.body, page) {
                pagination::Input::Move(target) => {
                    let pages = chats.len().div_ceil(PAGE_SIZE);
                    let Some(target) = target.filter(|target| *target < pages) else {
                        return reply(
                            tx,
                            job,
                            message!(message_catalog, "navigation.invalid_page", pages = pages),
                            now,
                        );
                    };
                    if target == page {
                        return Ok(());
                    }
                    page = target;
                    None
                }
                pagination::Input::Select(number) => Some(number),
                _ => {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.session_reply_04"),
                        now,
                    );
                }
            };
            if let Some(number) = number {
                let Some(chat) = number
                    .checked_sub(1)
                    .filter(|i| *i < PAGE_SIZE)
                    .and_then(|i| chats.get(page * PAGE_SIZE + i))
                else {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.session_reply_04"),
                        now,
                    );
                };
                if operation == "main" {
                    let mut settings = settings(tx, &context.square_id)?;
                    settings.main = Some(chat.chat_id.clone());
                    settings.source_chat = Some(identity(&job.event).1.into());
                    settings.bot_member = Some(context.bot_member_id.clone());
                    save_settings(tx, &context.square_id, &settings)?;
                } else {
                    if !save_notification(
                        tx,
                        &context.square_id,
                        &chat.chat_id,
                        &operation,
                        template,
                    )? {
                        return reply(
                            tx,
                            job,
                            message!(message_catalog, "commands.execute_16"),
                            now,
                        );
                    }
                }
                tx.execute("DELETE FROM oc_sessions WHERE id=?1", [id])?;
                if let Some(prompt) = prompt {
                    crate::enqueue_cleanup(
                        tx,
                        identity(&job.event).0,
                        identity(&job.event).1,
                        &prompt,
                        now,
                    )?;
                }
                return reply(
                    tx,
                    job,
                    message!(
                        message_catalog,
                        "commands.session_reply_05",
                        arg0 = chat.name
                    ),
                    now,
                );
            }
            let value = Session::Chats {
                chats,
                operation,
                template,
                page,
                page_size: PAGE_SIZE,
            };
            session(
                message_catalog,
                tx,
                job,
                value.clone(),
                chat_page(message_catalog, &value),
                now,
            )
        }
    }
}
fn case_reply(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &mut Job,
    now: i64,
) -> Result<()> {
    let id = job.input.args.first().ok_or("MissingOcCase")?.clone();
    let stored:Option<(String,String,Option<String>)>=tx.query_row("SELECT square,target,url FROM oc_cases WHERE id=?1 AND chat=?2 AND state='open' AND expires>?3",params![id,identity(&job.event).1,now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((square, target, url)) = stored else {
        return Ok(());
    };
    if square != job.context.as_ref().ok_or("MissingOcContext")?.square_id {
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.case_reply_01"),
            now,
        );
    };
    let input = job
        .input
        .body
        .nfkc()
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some(url) = url {
        let scope = match input.as_str() {
            "1" => "exact",
            "2" => "path",
            "3" => "prefix",
            "4" => "domain",
            "5" | "ignore" | "無視" => "",
            _ => {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.case_reply_02"),
                    now,
                );
            }
        };
        if !scope.is_empty() {
            let Some(rule) = policy::UrlRule::new(&url, scope) else {
                return reply(
                    tx,
                    job,
                    message!(message_catalog, "commands.case_reply_03"),
                    now,
                );
            };
            let mut value = settings(tx, &square)?;
            if !value.rules.contains(&rule) {
                if value.rules.len() >= 100 {
                    return reply(
                        tx,
                        job,
                        message!(message_catalog, "commands.execute_09"),
                        now,
                    );
                }
                value.rules.push(rule);
            }
            save_settings(tx, &square, &value)?;
        }
    } else if matches!(input.as_str(), "ban" | "再参加禁止" | "処分" | "キック") {
        job.operation = "case-ban".into();
        job.case_id = Some(id);
        job.targets = vec![target];
        return next_target(message_catalog, tx, job, now);
    } else if !matches!(
        input.as_str(),
        "ignore" | "無視" | "対応不要" | "不要" | "unban" | "解除" | "再参加禁止解除"
    ) {
        return reply(
            tx,
            job,
            message!(message_catalog, "commands.case_reply_04"),
            now,
        );
    }
    tx.execute(
        "UPDATE oc_cases SET state=?2 WHERE id=?1",
        params![
            id,
            if matches!(input.as_str(), "unban" | "解除" | "再参加禁止解除") {
                "unban-requested"
            } else {
                "resolved"
            }
        ],
    )?;
    reply(
        tx,
        job,
        message!(message_catalog, "commands.case_reply_05"),
        now,
    )
}
