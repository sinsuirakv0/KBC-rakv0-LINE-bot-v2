use super::*;
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
fn status(settings: &Settings) -> String {
    let flag = |enabled| if enabled { "ON" } else { "OFF" };
    format!(
        "OC管理設定\nURL許可制削除: {}（{}ルール）\n画像・動画連投: {}（30秒・7件目以降）\n即抜け監視: {}\n初参加・危険語: {}\n短時間一斉参加: {}\n危険語の通報: {}\n副官部屋: {}\n本OCトーク: {}\nミュート: {}人",
        flag(settings.url),
        settings.rules.len(),
        flag(settings.media),
        flag(settings.left),
        flag(settings.danger),
        flag(settings.cohort),
        flag(settings.report),
        settings.mod_room.as_deref().unwrap_or("未設定"),
        settings.main.as_deref().unwrap_or("未設定"),
        settings.mutes.len()
    )
}
fn setup_menu(settings: &Settings) -> String {
    format!(
        "OC管理セットアップ\nこのメッセージへ番号をリプライしてください。\n複数指定: 1 2 / 解除: off 1 2\n\n1 URL許可制削除\n2 画像・動画連投削除\n3 このトークを副官部屋に設定\n4 即抜け監視\n5 初参加・危険語処分\n6 短時間一斉参加監視\n7 基本項目をまとめてON（通報を除く）\n8 自動処理をOFF\n9 設定確認\n10 危険語処分時の通報\n11 本OCトークを選択\n終了: cancel\n\n{}",
        status(settings)
    )
}
pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let context = job.context.as_ref().ok_or("MissingOcContext")?.clone();
    let name = job.input.name.as_str();
    if name == "kicktest" {
        return reply(
            tx,
            job,
            "!oc kick を使用してください。confirmは不要です。",
            now,
        );
    }
    if name == "status" {
        return reply(tx, job, status(&settings(tx, &context.square_id)?), now);
    }
    if name == "authority" {
        return reply(
            tx,
            job,
            format!(
                "実行者のOC権限: {}\nBOT管理権限: {}\nBotのOC権限: {}\n\n{}",
                context.actor.role,
                bot_rank(runtime, job),
                context.bot_role,
                context.authority
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
    if !allowed(runtime, job, bot, oc) {
        return reply(
            tx,
            job,
            "実行権限がありません。!help oc で権限区分を確認してください。",
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
        return session_reply(tx, job, now);
    }
    if name == "case" {
        return case_reply(tx, job, now);
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
            "OC設定の保存上限に達しています。運用者へ確認してください。",
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
                return reply(tx, job, status(&value), now);
            }
            return session(tx, job, Session::Setup, setup_menu(&value), now);
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
                        "副官部屋の通知テストです。".into(),
                        TextDelivery::default(),
                        now,
                    )?;
                } else {
                    return reply(tx, job, "副官部屋が未設定です。!oc modroom set", now);
                }
                return reply(tx, job, "副官部屋のテスト送信を登録しました。", now);
            }
            _ => return reply(tx, job, "!oc modroom set / off / test", now),
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
                        "HTTPS URLと範囲を指定してください。\n!oc url add URL [exact|path|prefix|domain]",
                        now,
                    );
                };
                if !value.rules.contains(&rule) {
                    if value.rules.len() >= 100 {
                        return reply(tx, job, "URLルールは100件までです。", now);
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
                        "削除する番号を !oc url list で確認してください。",
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
                    .chain(std::iter::once(format!(
                        "ページ {page}/{}（!oc url list ページ番号）",
                        value.rules.len().div_ceil(5).max(1)
                    )))
                    .collect::<Vec<_>>()
                    .join("\n");
                return reply(
                    tx,
                    job,
                    if lines.is_empty() {
                        "URL許可ルールはありません。".into()
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
                    format!(
                        "URL監視: {} / {}ルール\n!oc url on / off\n!oc url add HTTPS-URL [exact|path|prefix|domain]\n!oc url list / remove 番号|all",
                        value.url,
                        value.rules.len()
                    ),
                    now,
                );
            }
        },
        "media" => match arg {
            "" | "on" => value.media = true,
            "off" | "del" => value.media = false,
            _ => return reply(tx, job, "!oc media on / off", now),
        },
        "watch" => {
            let enabled = match job.input.args.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => return reply(tx, job, "!oc watch early|danger|cohort|report on|off", now),
            };
            match arg {
                "early" => value.left = enabled,
                "danger" => value.danger = enabled,
                "cohort" => value.cohort = enabled,
                "report" => value.report = enabled,
                _ => return reply(tx, job, "!oc watch early|danger|cohort|report on|off", now),
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
                        None => "入退室メッセージは未設定です。".into(),
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
                        "通知本文を1〜1300文字で指定してください。\n!oc join set [--mention] [--id] 本文",
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
                    "入退室通知の保存上限に達しています。運用者へ確認してください。",
                    now,
                );
            }
            return reply(tx, job, "このトークの入退室メッセージを保存しました。", now);
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
                        format!(
                            "{} ({id})\n期限: {}",
                            mute.name,
                            mute.until
                                .map(|at| format!("{} JST", jst(at)))
                                .unwrap_or("無期限".into())
                        )
                    })
                    .chain(std::iter::once(format!(
                        "ページ {page}/{}（!oc mute list ページ番号）",
                        value.mutes.len().div_ceil(20).max(1)
                    )))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                return reply(
                    tx,
                    job,
                    if text.is_empty() {
                        "ミュート中のメンバーはいません。".into()
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
                    "!oc mute @対象 170 / 0:17 / 8/7-0:17 / inf\n!oc mute @対象 off / list",
                    now,
                );
            }
            return next_target(tx, job, now);
        }
        "kick" => {
            if matches!(arg, "his" | "history") {
                return show_history(tx, job, now);
            }
            job.operation = "manual-ban".into();
            job.targets = targets(job);
            if job.targets.is_empty() || job.targets.len() > 8 {
                return reply(
                    tx,
                    job,
                    "対象をメンションまたはMIDで指定してください。1回8人までです。\n!oc kick @対象 理由",
                    now,
                );
            }
            return next_target(tx, job, now);
        }
        "history" => return show_history(tx, job, now),
        _ => return reply(tx, job, "使い方: !help oc / !oc adminhelp", now),
    }
    save_settings(tx, &context.square_id, &value)?;
    reply(tx, job, status(&value), now)
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
fn targets(job: &Job) -> Vec<String> {
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
pub(super) fn reason(job: &Job) -> String {
    match job.operation.as_str() {
        "danger-kick" => return "初参加から2分以内の危険語".into(),
        "left-ban" => return "初参加から5分以内のOC全体退出".into(),
        "case-ban" => return "副官審議による再参加禁止".into(),
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
pub fn next_target(tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
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
            format!(
                "強制退会・再参加禁止結果\n{}\n理由: {}",
                job.results.join("\n"),
                if reason(job).is_empty() {
                    "未指定".into()
                } else {
                    reason(job)
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
                format!(
                    "{}\n実行トーク: {}\n実行者: {} ({})\n{text}",
                    if job.operation == "manual-ban" {
                        "【手動処分】OC再参加禁止"
                    } else {
                        "【OC管理結果】"
                    },
                    identity(&job.event).1,
                    context.actor.name,
                    context.actor.member_id
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
    let member = result
        .oc_result
        .as_ref()
        .and_then(|value| value.member.clone())
        .ok_or("MissingOcMember")?;
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    if member.square_id != context.square_id || job.targets.first() != Some(&member.member_id) {
        return reply(tx, job, "対象メンバーのOCが一致しません。", now);
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
                return reply(tx, job, "ミュートはOCごと100人までです。", now);
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
                "期限を確認してください。170 / 0:17 / 8/7-0:17 / inf / off",
                now,
            );
        }
        save_settings(tx, &context.square_id, &value)?;
        history(tx, job, &member.member_id, "saved", duration, now)?;
        let period = value.mutes.get(&member.member_id).map(|mute| {
            mute.until.map_or("無期限".into(), |until| {
                format!(
                    "{} JSTまで（残り{}）",
                    jst(until),
                    policy::duration(until - now)
                )
            })
        });
        let text = period.as_ref().map_or(
            format!("{} のミュートを解除しました。", member.name),
            |period| {
                format!(
                    "{} をミュートしました。\n期間: {period}\n対象の新規発言は削除されます。",
                    member.name
                )
            },
        );
        if let Some(room) = value.mod_room.filter(|room| room != identity(&job.event).1) {
            text_action(
                tx,
                &job.event,
                "mute-log",
                &room,
                format!(
                    "【OCミュート】\n実行トーク: {}\n実行者: {} ({})\n対象: {} ({})\n{text}",
                    identity(&job.event).1,
                    context.actor.name,
                    context.actor.member_id,
                    member.name,
                    member.member_id
                ),
                TextDelivery::default(),
                now,
            )?;
        }
        return reply(tx, job, text, now);
    }
    if member.member_id == context.bot_member_id
        || policy::role_rank(&member.role) != 1
        || runtime
            .permissions
            .rank(&member.square_id, &member.member_id)
            .max(
                runtime
                    .permissions
                    .rank(identity(&job.event).1, &member.member_id),
            )
            > 0
    {
        job.results.push(format!(
            "{} ({}): 権限未確認または保護対象のため未処分",
            member.name, member.member_id
        ));
        job.targets.remove(0);
        return next_target(tx, job, now);
    }
    if job.operation == "left-ban" && !matches!(member.state.as_str(), "LEFT" | "4") {
        return Ok(());
    }
    if job.operation == "left-check" {
        return moderation::confirm_left(tx, job, &member, now);
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
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let target = job.targets.first().cloned().ok_or("MissingOcTarget")?;
    let label = match result.status {
        DeliveryStatus::Sent => "成功",
        DeliveryStatus::Unknown => "結果不明（自動再試行しません）",
        DeliveryStatus::Failed => "失敗",
    };
    history(
        tx,
        job,
        &target,
        label,
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
        .map_or("メンバー", |member| member.name.as_str());
    job.results.push(format!(
        "{label}: {name} ({target}){}",
        if result.code == "OK" {
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
                policy::danger_word(text).unwrap_or("未取得"),
                text.chars().take(300).collect::<String>(),
            )
        } else {
            ("未取得", String::new())
        };
        let elapsed = joined.map_or("未取得".into(), |joined| {
            policy::duration(identity(&job.event).4 - joined)
        });
        moderation::case(
            tx,
            job,
            Some(&target),
            None,
            &format!(
                "【自動処分】初参加直後の危険語\n参加から: {elapsed}\n検出語: {word}\n本文: {body}\n処分: メッセージ削除 + 強制退会（再参加禁止は未実行）\n結果: {label}\n{}\n誤検知の可能性があります。再参加禁止 / 無視 / 解除で審議できます。",
                job.results.join("\n")
            ),
            now,
        )?;
        return reply(
            tx,
            job,
            format!(
                "スパムフィルターによる強制退会: {label}\n参加直後に「チート」「代行」を含む投稿を検知しました。宣伝・不正行為勧誘の可能性があると判定しました。誤検知の可能性もあるため、副官がログを確認してください。"
            ),
            now,
        );
    }
    if job.operation == "left-ban" {
        moderation::case(
            tx,
            job,
            Some(&target),
            None,
            &format!(
                "【自動処分】参加後5分以内の退会\n処分: 再参加禁止\n結果: {label}\n{}",
                job.results.join("\n")
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
                format!(
                    "自動再参加禁止: {label}\n判断理由: 初参加から5分以内の即抜け\n{}",
                    job.results.join("\n")
                ),
                TextDelivery::default(),
                now,
            )?;
        }
        return Ok(());
    }
    next_target(tx, job, now)
}
fn show_history(tx: &Transaction<'_>, job: &Job, now: i64) -> Result<()> {
    let square = &job.context.as_ref().ok_or("MissingOcContext")?.square_id;
    let kick_only = job.input.name == "kick";
    let mut query=tx.prepare("SELECT target,operation,status,detail,at,target_name,actor_name,reason,actor FROM oc_history WHERE square=?1 AND (?2=0 OR operation IN ('manual-ban','danger-kick','left-ban','case-ban')) ORDER BY id DESC LIMIT ?3")?;
    let rows = query
        .query_map(
            params![square, kick_only, if kick_only { 10 } else { 15 }],
            |r| {
                Ok(format!(
                    "{} {}\n操作: {}\n対象: {} ({})\n実行者: {} ({})\n理由: {}\n詳細: {}",
                    jst(r.get(4)?),
                    r.get::<_, String>(2)?,
                    match r.get::<_, String>(1)?.as_str() {
                        "danger-kick" => "強制退会",
                        "manual-ban" => "強制退会 + 再参加禁止",
                        "left-ban" | "case-ban" => "再参加禁止",
                        "bot-name" => "Botの名前変更",
                        operation => operation,
                    }
                    .to_owned(),
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(3)?
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    reply(
        tx,
        job,
        if rows.is_empty() {
            "処分・操作履歴はありません。".into()
        } else {
            format!(
                "{}\n\n{}",
                if kick_only {
                    "強制退会・再参加禁止履歴"
                } else {
                    "OC操作履歴"
                },
                rows.join("\n\n")
            )
        },
        now,
    )
}
fn jst(at: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at + 9 * 3600000)
        .map(|date| date.format("%Y/%m/%d %H:%M").to_string())
        .unwrap_or("不明".into())
}
pub fn chats(tx: &Transaction<'_>, job: &mut Job, result: &ActionResult, now: i64) -> Result<()> {
    let chats = result
        .oc_result
        .as_ref()
        .ok_or("MissingOcChats")?
        .chats
        .clone();
    if chats.is_empty() || chats.len() > 64 {
        return reply(
            tx,
            job,
            "候補を取得できませんでした。対象トークから直接設定してください。",
            now,
        );
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
    };
    session(tx, job, value.clone(), chat_page(&value), now)
}
fn chat_page(value: &Session) -> String {
    let Session::Chats { chats, page, .. } = value else {
        return String::new();
    };
    let mut lines = vec!["送信先を選び、このメッセージへ番号をリプライしてください。".into()];
    for (index, chat) in chats.iter().skip(page * 8).take(8).enumerate() {
        lines.push(format!(
            "{} {}{}",
            index + 1,
            if chat.is_main { "本OC: " } else { "" },
            chat.name
        ));
    }
    if (page + 1) * 8 < chats.len() {
        lines.push("9 次へ".into());
    }
    if *page > 0 {
        lines.push("0 前へ".into());
    }
    lines.push("cancel 終了".into());
    lines.join("\n")
}
fn session_reply(tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
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
        return reply(tx, job, "OC設定の操作を終了しました。", now);
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
                    "番号1〜11を指定してください。例: 1 2 / off 1 2",
                    now,
                );
            };
            if numbers.contains(&11) {
                if numbers.len() != 1 {
                    return reply(tx, job, "本OCの選択は11だけで指定してください。", now);
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
            session(tx, job, Session::Setup, setup_menu(&settings), now)
        }
        Session::Chats {
            chats,
            operation,
            template,
            mut page,
        } => {
            let number = body.parse::<usize>().unwrap_or(usize::MAX);
            if number == 9 && (page + 1) * 8 < chats.len() {
                page += 1;
            } else if number == 0 && page > 0 {
                page -= 1;
            } else {
                let Some(chat) = number
                    .checked_sub(1)
                    .filter(|i| *i < 8)
                    .and_then(|i| chats.get(page * 8 + i))
                else {
                    return reply(tx, job, "表示中の候補番号を指定してください。", now);
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
                            "入退室通知の保存上限に達しています。運用者へ確認してください。",
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
                    format!("{} の設定を保存しました。", chat.name),
                    now,
                );
            }
            let value = Session::Chats {
                chats,
                operation,
                template,
                page,
            };
            session(tx, job, value.clone(), chat_page(&value), now)
        }
    }
}
fn case_reply(tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let id = job.input.args.first().ok_or("MissingOcCase")?.clone();
    let stored:Option<(String,String,Option<String>)>=tx.query_row("SELECT square,target,url FROM oc_cases WHERE id=?1 AND chat=?2 AND state='open' AND expires>?3",params![id,identity(&job.event).1,now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((square, target, url)) = stored else {
        return Ok(());
    };
    if square != job.context.as_ref().ok_or("MissingOcContext")?.square_id {
        return reply(tx, job, "このOCの審議ではありません。", now);
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
                    "URL審議は1 完全一致 / 2 パス / 3 配下 / 4 ドメイン / 5 却下",
                    now,
                );
            }
        };
        if !scope.is_empty() {
            let Some(rule) = policy::UrlRule::new(&url, scope) else {
                return reply(
                    tx,
                    job,
                    "HTTPS以外のURLは許可できません。5で却下できます。",
                    now,
                );
            };
            let mut value = settings(tx, &square)?;
            if !value.rules.contains(&rule) {
                if value.rules.len() >= 100 {
                    return reply(tx, job, "URLルールは100件までです。", now);
                }
                value.rules.push(rule);
            }
            save_settings(tx, &square, &value)?;
        }
    } else if matches!(input.as_str(), "ban" | "再参加禁止" | "処分" | "キック") {
        job.operation = "case-ban".into();
        job.case_id = Some(id);
        job.targets = vec![target];
        return next_target(tx, job, now);
    } else if !matches!(
        input.as_str(),
        "ignore" | "無視" | "対応不要" | "不要" | "unban" | "解除" | "再参加禁止解除"
    ) {
        return reply(
            tx,
            job,
            "審議は 再参加禁止 / 無視 / 解除（依頼の記録のみ）",
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
        "審議を記録しました。削除済み投稿の復元や再参加禁止の自動解除は行いません。",
        now,
    )
}
