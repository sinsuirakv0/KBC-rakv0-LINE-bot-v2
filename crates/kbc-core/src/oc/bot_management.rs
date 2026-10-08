use super::*;
use crate::{messages::message, permissions};

pub fn is_start(text: &str) -> bool {
    bot::parse(text).is_some_and(|input| {
        input.name == "bot"
            && input
                .args
                .first()
                .is_some_and(|arg| arg.eq_ignore_ascii_case("start"))
    })
}
pub fn is_start_job(job: &Job) -> bool {
    job.input.name == "bot"
        && job
            .input
            .args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case("start"))
}

// 状態確認は権限変更を伴わないため、LINE照会を待たず保存済み情報だけで返す。
pub fn quick_status(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    event: &CoreEvent,
    text: &str,
    now: i64,
) -> Result<bool> {
    if !bot::parse(text).is_some_and(|input| {
        input.name == "bot" && input.args.len() == 1 && input.args[0].eq_ignore_ascii_case("status")
    }) {
        return Ok(false);
    }
    status(
        runtime,
        tx,
        event,
        message!(&runtime.content.messages, "common.unavailable"),
        now,
    )?;
    Ok(true)
}
fn status(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    event: &CoreEvent,
    oc_role: &str,
    now: i64,
) -> Result<()> {
    let catalog = &runtime.content.messages;
    let stats = runtime.stats_from_db(tx)?;
    let (all, local) = permissions::stop_state(tx, identity(event).1)?;
    let last: Option<i64> =
        tx.query_row("SELECT max(received) FROM events", [], |row| row.get(0))?;
    let body = message!(
        catalog,
        "bot.status",
        state = if all || local {
            message!(catalog, "bot.stopped")
        } else {
            message!(catalog, "bot.running")
        },
        all = flag(catalog, all),
        local = flag(catalog, local),
        uptime = (now - runtime.started_at_ms).max(0) / 1000,
        role = role(
            catalog,
            permissions::rank(
                tx,
                square(event).unwrap_or(""),
                identity(event).1,
                identity(event).3
            )?
        ),
        oc_role = oc_role,
        received = last.map_or_else(
            || message!(catalog, "common.not_set").into(),
            |at| format!("{}", (now - at).max(0) / 1000)
        ),
        waiting = stats.queued_actions,
        queries = stats.querying_actions,
        sending = stats.claimed_actions + stats.sending_actions,
        unknown = stats.unknown_actions,
        media = stats.preparing_media,
        logs = stats.pending_logs
    );
    text_action(
        tx,
        event,
        "bot-status",
        identity(event).1,
        body,
        TextDelivery::default(),
        now,
    )?;
    Ok(())
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let catalog = &runtime.content.messages;
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    let args = &job.input.args;
    let action = args.first().map_or("", String::as_str).to_ascii_lowercase();
    match action.as_str() {
        "status" if args.len() == 1 => status(runtime, tx, &job.event, &context.actor.role, now),
        "start" | "stop"
            if args.len() == 1 || (args.len() == 2 && args[1].eq_ignore_ascii_case("all")) =>
        {
            let all = args.len() == 2;
            if bot_rank(tx, job)? < 2 && (all || policy::role_rank(&context.actor.role) < 2) {
                return reply(tx, job, message!(catalog, "bot.control_denied"), now);
            }
            let chat = if all { "*" } else { identity(&job.event).1 };
            if action == "stop"
                && !permissions::stopped(tx, chat)?
                && tx.query_row("SELECT count(*) FROM bot_stops", [], |row| {
                    row.get::<_, i64>(0)
                })? >= 2048
            {
                return reply(tx, job, message!(catalog, "bot.capacity"), now);
            }
            let changed =
                permissions::control(tx, chat, action == "stop", identity(&job.event).3, now)?;
            job.operation = format!("bot-{action}");
            history(
                catalog,
                tx,
                job,
                chat,
                "成功",
                &format!("all={all},changed={changed}"),
                now,
            )?;
            reply(
                tx,
                job,
                message!(
                    catalog,
                    "bot.control_result",
                    action = if action == "stop" {
                        message!(catalog, "bot.stop")
                    } else {
                        message!(catalog, "bot.start")
                    },
                    scope = chat,
                    changed = if changed {
                        message!(catalog, "common.success")
                    } else {
                        message!(catalog, "bot.unchanged")
                    },
                    all = flag(
                        catalog,
                        permissions::stop_state(tx, identity(&job.event).1)?.0
                    )
                ),
                now,
            )
        }
        "admin" => list(runtime, tx, job, now),
        "setting"
            if args
                .get(1)
                .is_some_and(|arg| arg.eq_ignore_ascii_case("status")) =>
        {
            let Some((scope, alias)) = scope(job, &args[2..]) else {
                return help(runtime, tx, job, now);
            };
            reply(
                tx,
                job,
                message!(
                    catalog,
                    "bot.permission_status",
                    scope = scope,
                    role = role(
                        catalog,
                        permissions::rank(tx, &scope, &alias, identity(&job.event).3)?
                    ),
                    oc_role = context.actor.role
                ),
                now,
            )
        }
        "setting"
            if args.get(1).is_some_and(|arg| {
                matches!(arg.to_ascii_lowercase().as_str(), "admin" | "mod")
            }) =>
        {
            grant(runtime, tx, job, now)
        }
        _ => help(runtime, tx, job, now),
    }
}

fn help(runtime: &Runtime, tx: &Transaction<'_>, job: &Job, now: i64) -> Result<()> {
    reply(
        tx,
        job,
        runtime.content.command_help("bot").unwrap_or_default(),
        now,
    )
}

fn valid_mid(value: &str, kind: char) -> bool {
    value.len() >= 9
        && value.len() <= 64
        && value.starts_with(kind)
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn scope(job: &Job, args: &[String]) -> Option<(String, String)> {
    let context = job.context.as_ref()?;
    let mut explicit = args.iter().filter_map(|arg| {
        arg.split_once(':')
            .filter(|(key, _)| key.eq_ignore_ascii_case("talkID"))
            .map(|(_, value)| value)
    });
    if let Some(value) = explicit.next() {
        if explicit.next().is_some() || !(valid_mid(value, 's') || valid_mid(value, 'm')) {
            return None;
        }
        let value = value.to_ascii_lowercase();
        return Some((value.clone(), value));
    }
    Some((context.square_id.clone(), identity(&job.event).1.into()))
}
fn role(catalog: &crate::messages::Messages, rank: u8) -> &str {
    match rank {
        2 => message!(catalog, "bot.admin"),
        1 => message!(catalog, "bot.mod"),
        _ => message!(catalog, "bot.member"),
    }
}
fn flag(catalog: &crate::messages::Messages, enabled: bool) -> &str {
    if enabled {
        message!(catalog, "bot.on")
    } else {
        message!(catalog, "bot.off")
    }
}

fn grant(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let catalog = &runtime.content.messages;
    let Some((scope, alias)) = scope(job, &job.input.args[2..]) else {
        return help(runtime, tx, job, now);
    };
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    // OC管理人による委任は実行OCだけ。BOT管理者の遠隔設定と混ぜない。
    if bot_rank(tx, job)? < 2
        && !(policy::role_rank(&context.actor.role) == 3
            && (scope == context.square_id || scope == identity(&job.event).1))
    {
        return reply(tx, job, message!(catalog, "bot.roles_denied"), now);
    }
    let targets = commands::targets(job);
    let Some(member) = targets.first().map(|id| id.to_ascii_lowercase()) else {
        return reply(tx, job, message!(catalog, "bot.target_required"), now);
    };
    let role_name = job.input.args[1].to_ascii_lowercase();
    let remove = job
        .input
        .args
        .iter()
        .any(|arg| arg.eq_ignore_ascii_case("del"));
    if !remove
        && permissions::rank(tx, &scope, &alias, &member)? == 0
        && tx.query_row("SELECT count(*) FROM bot_roles", [], |row| {
            row.get::<_, i64>(0)
        })? >= 4096
    {
        return reply(tx, job, message!(catalog, "bot.capacity"), now);
    }
    let changed = permissions::set_role(
        tx,
        &scope,
        &alias,
        &member,
        &role_name,
        remove,
        (identity(&job.event).3, now),
    )?;
    job.operation = format!("bot-role-{role_name}");
    history(
        catalog,
        tx,
        job,
        &member,
        "成功",
        &format!("scope={scope},remove={remove},changed={changed}"),
        now,
    )?;
    reply(
        tx,
        job,
        message!(
            catalog,
            "bot.role_result",
            role = role(catalog, if role_name == "admin" { 2 } else { 1 }),
            scope = scope,
            member = member,
            operation = if remove {
                message!(catalog, "bot.remove")
            } else {
                message!(catalog, "bot.register")
            },
            changed = if changed {
                message!(catalog, "common.success")
            } else {
                message!(catalog, "bot.unchanged")
            }
        ),
        now,
    )
}

fn list(runtime: &Runtime, tx: &Transaction<'_>, job: &Job, now: i64) -> Result<()> {
    let catalog = &runtime.content.messages;
    let Some((scope, alias)) = scope(job, &job.input.args[1..]) else {
        return help(runtime, tx, job, now);
    };
    let pages: Vec<_> = job.input.args[1..]
        .iter()
        .filter_map(|arg| arg.strip_suffix('p'))
        .collect();
    let page = match pages.as_slice() {
        [] => 1,
        [value] => match value.parse::<usize>() {
            Ok(page) if (1..=410).contains(&page) => page,
            _ => return help(runtime, tx, job, now),
        },
        _ => return help(runtime, tx, job, now),
    };
    let count: u32 = tx.query_row(
        "SELECT count(DISTINCT member) FROM bot_roles WHERE scope IN (?1,?2)",
        params![scope, alias],
        |row| row.get(0),
    )?;
    let total = (count as usize).div_ceil(10).max(1);
    if page > total {
        return help(runtime, tx, job, now);
    }
    // 一覧のために全員のプロフィールを取得しない。観測名があれば使う。
    let mut statement = tx.prepare("SELECT r.member,max(CASE r.role WHEN 'admin' THEN 2 ELSE 1 END),COALESCE((SELECT name FROM oc_members WHERE square IN (?1,?4) AND member=r.member ORDER BY last DESC LIMIT 1),'') FROM bot_roles r WHERE r.scope IN (?1,?2) GROUP BY r.member ORDER BY 2 DESC,r.member LIMIT 10 OFFSET ?3")?;
    let rows = statement
        .query_map(
            params![
                scope,
                alias,
                ((page - 1) * 10) as i64,
                job.context.as_ref().ok_or("MissingOcContext")?.square_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u8>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let body = rows
        .into_iter()
        .map(|(member, rank, name)| {
            message!(
                catalog,
                "bot.role_entry",
                role = role(catalog, rank),
                name = name.chars().take(32).collect::<String>(),
                member = member
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    reply(
        tx,
        job,
        message!(
            catalog,
            "bot.role_list",
            scope = scope,
            body = if body.is_empty() {
                message!(catalog, "bot.none").into()
            } else {
                body
            },
            page = page,
            total = total
        ),
        now,
    )
}
