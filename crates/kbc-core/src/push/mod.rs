//! 通知設定と予約を既存transaction・Session・Outboxへ接続する。
use crate::{
    Result,
    commands::sessions::Response,
    messages::{Messages, message},
};
use kbc_protocol::{CoreAction, CoreEvent, MessageMention};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

mod monitor;
mod schedule;
#[cfg(test)]
mod tests;
mod time;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Choice {
    pub kind: String,
    pub id: i64,
    pub name: String,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Selection {
    pub choices: Vec<Choice>,
    pub page: usize,
    pub advance: i64,
    pub enabled: bool,
}

pub(crate) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS push_destinations(chat TEXT PRIMARY KEY,checked INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS push_subscriptions(chat TEXT NOT NULL,kind TEXT NOT NULL,target INTEGER NOT NULL,advance INTEGER NOT NULL,since INTEGER NOT NULL,PRIMARY KEY(chat,kind,target));
        CREATE TABLE IF NOT EXISTS push_deliveries(id TEXT PRIMARY KEY,chat TEXT NOT NULL,due INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS push_deliveries_due ON push_deliveries(due);")?;
    Ok(())
}

pub(crate) fn configure(
    tx: &Transaction<'_>,
    chat: &str,
    choice: &Choice,
    advance: i64,
    enabled: bool,
    messages: &Messages,
    now: i64,
) -> Result<String> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM push_subscriptions WHERE chat=?1 AND kind=?2 AND target=?3)",
        params![chat, choice.kind, choice.id],
        |r| r.get(0),
    )?;
    if enabled && !exists {
        let total: i64 =
            tx.query_row("SELECT count(*) FROM push_subscriptions", [], |r| r.get(0))?;
        let local: i64 = tx.query_row(
            "SELECT count(*) FROM push_subscriptions WHERE chat=?1",
            [chat],
            |r| r.get(0),
        )?;
        let chats: i64 =
            tx.query_row("SELECT count(*) FROM push_destinations", [], |r| r.get(0))?;
        let registered: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM push_destinations WHERE chat=?1)",
            [chat],
            |r| r.get(0),
        )?;
        if total >= 2048 || local >= 128 || (!registered && chats >= 512) {
            return Ok(message!(messages, "push.capacity").into());
        }
    }
    if enabled {
        tx.execute(
            "INSERT INTO push_destinations VALUES(?1,?2) ON CONFLICT(chat) DO NOTHING",
            params![chat, now],
        )?;
        tx.execute("INSERT INTO push_subscriptions VALUES(?1,?2,?3,?4,?5) ON CONFLICT(chat,kind,target) DO UPDATE SET advance=excluded.advance,since=excluded.since",params![chat,choice.kind,choice.id,advance,now])?;
    } else {
        tx.execute(
            "DELETE FROM push_subscriptions WHERE chat=?1 AND kind=?2 AND target=?3",
            params![chat, choice.kind, choice.id],
        )?;
    }
    // 1分先までの未送信予定を作り直す。通信済み・結果不明は取り消さない。
    tx.execute("DELETE FROM actions WHERE chat=?1 AND event_id LIKE 'push:event:%' AND due>?2 AND status='queued' AND json_extract(payload,'$.threadRootId') IS NULL",params![chat,now])?;
    if !enabled && choice.kind == "daily" {
        tx.execute("DELETE FROM actions WHERE chat=?1 AND event_id LIKE 'push:event:%:daily' AND status='queued'",[chat])?;
    }
    tx.execute("DELETE FROM push_deliveries WHERE chat=?1 AND due>?2 AND NOT EXISTS(SELECT 1 FROM actions WHERE actions.event_id=push_deliveries.id)",params![chat,now])?;
    tx.execute(
        "UPDATE push_destinations SET checked=min(checked,?2) WHERE chat=?1",
        params![chat, now],
    )?;
    tx.execute("DELETE FROM push_destinations WHERE NOT EXISTS(SELECT 1 FROM push_subscriptions s WHERE s.chat=push_destinations.chat)",[])?;
    Ok(message!(
        messages,
        "push.configured",
        target = display_target(messages, choice),
        state = if enabled {
            message!(messages, "update.on")
        } else {
            message!(messages, "update.off")
        },
        advance = advance
    ))
}

pub(crate) fn apply(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    args: &[String],
    messages: &Messages,
    now: i64,
) -> Result<Vec<Response>> {
    let mut normalized = args.to_vec();
    if normalized
        .first()
        .is_some_and(|a| a.eq_ignore_ascii_case("event"))
    {
        for value in &mut normalized {
            if ["event", "g", "all", "daily", "del", "off"]
                .iter()
                .any(|word| value.eq_ignore_ascii_case(word))
            {
                *value = value.to_ascii_lowercase();
            }
        }
    } else if normalized
        .first()
        .is_some_and(|a| a.eq_ignore_ascii_case("status"))
    {
        normalized[0] = "status".into();
    }
    let args = &normalized;
    let (event_id, chat, _, actor, _) = crate::oc::identity(event);
    let reply = |text: String| vec![(text, now, None)];
    if args.first().is_some_and(|a| a == "status") {
        let page = if args.len() == 1 {
            0
        } else if args.len() == 2 {
            match crate::commands::pagination::parse(&args[1], 0) {
                crate::commands::pagination::Input::Move(Some(page)) => page,
                _ => return Ok(reply(message!(messages, "push.status_hint").into())),
            }
        } else {
            return Ok(reply(message!(messages, "push.status_hint").into()));
        };
        let settings = tx.prepare("SELECT kind,target,advance FROM push_subscriptions WHERE chat=?1 ORDER BY kind,target")?
            .query_map([chat],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        let pages = settings.len().div_ceil(10).max(1);
        if page >= pages {
            return Ok(reply(message!(
                messages,
                "navigation.invalid_page",
                pages = pages
            )));
        }
        let mut lines = settings
            .into_iter()
            .skip(page * 10)
            .take(10)
            .map(|(kind, id, advance)| {
                message!(
                    messages,
                    "push.status_entry",
                    kind = kind_label(messages, &kind),
                    id = if matches!(kind.as_str(), "all" | "g-all" | "daily") {
                        String::new()
                    } else {
                        id.to_string()
                    },
                    advance = advance
                )
            })
            .collect::<Vec<_>>();
        let pending: i64 = tx.query_row("SELECT count(DISTINCT event_id) FROM actions WHERE chat=?1 AND id LIKE 'push:reminder:%' AND status NOT IN ('sent','failed')",[chat],|r|r.get(0))?;
        lines.push(message!(messages, "push.reminder_count", count = pending));
        lines.push(message!(
            messages,
            "push.status_page",
            page = page + 1,
            pages = pages
        ));
        return Ok(crate::commands::split_text(&lines.join("\n"))
            .into_iter()
            .take(8)
            .map(|text| (text, now, None))
            .collect());
    }
    if args.first().is_some_and(|a| a == "event") {
        let rest = &args[1..];
        if matches!(rest.first().map(String::as_str), Some("del" | "off")) && rest.len() == 1 {
            tx.execute("DELETE FROM push_subscriptions WHERE chat=?1", [chat])?;
            tx.execute("DELETE FROM push_destinations WHERE chat=?1", [chat])?;
            tx.execute("DELETE FROM actions WHERE chat=?1 AND event_id LIKE 'push:event:%' AND status='queued'",[chat])?;
            return Ok(reply(message!(messages, "push.removed").into()));
        }
        if rest.is_empty() {
            return Ok(reply(message!(messages, "push.event_hint").into()));
        }
        let mut rest = rest;
        let gacha = rest[0] == "g";
        if gacha {
            rest = &rest[1..];
        }
        let mut enabled = true;
        let mut advance = 0;
        if let Some(last) = rest.last() {
            if matches!(last.as_str(), "del" | "off") {
                enabled = false;
                rest = &rest[..rest.len() - 1];
            } else if let Some(number) = last.strip_prefix('-') {
                match number.parse::<i64>() {
                    Ok(value @ 1..=525_600) => {
                        advance = value;
                        rest = &rest[..rest.len() - 1];
                    }
                    _ => return Ok(reply(message!(messages, "push.invalid_advance").into())),
                }
            }
        }
        let direct = if gacha && rest.is_empty() {
            Some(Choice {
                kind: "g-all".into(),
                id: 0,
                name: message!(messages, "push.all_gacha").into(),
            })
        } else if !gacha && rest == ["all"] {
            Some(Choice {
                kind: "all".into(),
                id: 0,
                name: message!(messages, "push.all_event").into(),
            })
        } else if !gacha && rest == ["daily"] {
            Some(Choice {
                kind: "daily".into(),
                id: 0,
                name: message!(messages, "push.daily").into(),
            })
        } else if rest.len() == 1
            && let Ok(id @ 0..=2_147_483_647) = rest[0].parse::<i64>()
        {
            Some(Choice {
                kind: if gacha { "g" } else { "sale" }.into(),
                id,
                name: String::new(),
            })
        } else {
            None
        };
        if let Some(choice) = direct {
            if advance > 0 && matches!(choice.kind.as_str(), "all" | "daily") {
                return Ok(reply(message!(messages, "push.invalid_event").into()));
            }
            return Ok(reply(configure(
                tx, chat, &choice, advance, enabled, messages, now,
            )?));
        }
        if rest.is_empty() || rest.join(" ").len() > 512 || rest.len() > 16 {
            return Ok(reply(message!(messages, "push.invalid_event").into()));
        }
        let mut arguments = vec![
            if gacha { "g" } else { "sale" }.into(),
            advance.to_string(),
            enabled.to_string(),
        ];
        arguments.extend(rest.iter().cloned());
        return Ok(vec![(
            String::new(),
            now,
            Some(crate::media::MediaJob {
                catalog_revision: String::new(),
                request: crate::media::MediaRequest::EventData(crate::event_data::Request {
                    command: "push".into(),
                    arguments,
                    owner: (!actor.is_empty()).then(|| actor.into()),
                }),
            }),
        )]);
    }
    let (due, body) = match time::parse(args, now) {
        Ok(value) => value,
        Err(key) => return Ok(reply(messages.literal(key).into())),
    };
    if crate::skd::pending_count(tx)? >= crate::MAX_ACTIONS - 8 {
        return Ok(reply(message!(messages, "push.capacity").into()));
    }
    let label = match event {
        CoreEvent::MessageReceived { sender_name, .. } => sender_name
            .as_deref()
            .unwrap_or(message!(messages, "common.member")),
        _ => message!(messages, "common.member"),
    };
    let prefix = message!(
        messages,
        "push.reminder_label",
        name = label.chars().take(80).collect::<String>()
    );
    let content = if body.is_empty() {
        message!(messages, "push.reminder_default")
    } else {
        &body
    };
    let parts = crate::commands::split_text(&format!("{prefix}\n{content}"));
    if parts.len() > 8 {
        return Ok(reply(message!(messages, "push.content_limit").into()));
    }
    // 長期予約だけで共通Outboxを埋めないよう、予約の未解決枠は半分までにする。
    let reserved: i64 = tx.query_row("SELECT count(*) FROM actions WHERE id LIKE 'push:reminder:%' AND status NOT IN ('sent','failed')",[],|r|r.get(0))?;
    if reserved + parts.len() as i64 > crate::MAX_ACTIONS / 2 {
        return Ok(reply(message!(messages, "push.capacity").into()));
    }
    for (index, text) in parts.into_iter().enumerate() {
        let id = format!("push:reminder:{event_id}");
        let mut action = notification(id.clone(), chat, text, now);
        if let CoreAction::SendMessage {
            action_id,
            event_id: source_event,
            ..
        } = &mut action
        {
            *action_id = format!("{id}:{index}");
            // 元の受信IDを保持し、未来の予約が残る間は既存の重複ID清掃から保護する。
            *source_event = event_id.into();
        }
        if index == 0
            && !actor.is_empty()
            && let CoreAction::SendMessage { mention, .. } = &mut action
        {
            *mention = Some(MessageMention {
                member_id: actor.into(),
                start: 0,
                end: prefix.encode_utf16().count() as u32,
                additional: None,
            });
        }
        insert(tx, &action, due)?;
    }
    Ok(reply(message!(
        messages,
        "push.reminder_saved",
        date = time::format_time(due)
    )))
}

pub(super) fn notification(id: String, chat: &str, text: String, now: i64) -> CoreAction {
    CoreAction::SendMessage {
        action_id: format!("{id}:0"),
        event_id: id,
        chat_id: chat.into(),
        related_message_id: String::new(),
        text,
        thread_root_id: None,
        thread_contents: None,
        mention: None,
        emojis: None,
        image_url: None,
        attachment: None,
        replace_message_id: None,
        is_prompt: false,
        created_at_ms: now,
    }
}
pub(super) fn insert(tx: &Transaction<'_>, action: &CoreAction, due: i64) -> Result<()> {
    if let CoreAction::SendMessage {
        action_id,
        event_id,
        chat_id,
        created_at_ms,
        ..
    } = action
    {
        tx.execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,?5,?6,'queued')",params![action_id,event_id,chat_id,serde_json::to_string(action)?,due,created_at_ms])?;
    }
    Ok(())
}

pub(super) fn kind_label<'a>(messages: &'a Messages, kind: &str) -> &'a str {
    messages.literal(match kind {
        "sale" => "push.event_kind",
        "g" => "push.gacha_kind",
        "gr" => "push.rare_kind",
        "ge" => "push.event_gacha_kind",
        "gn" => "push.normal_kind",
        "all" => "push.all_event",
        "g-all" => "push.all_gacha",
        "daily" => "push.daily",
        _ => "push.event_kind",
    })
}

fn display_target(messages: &Messages, choice: &Choice) -> String {
    let label = kind_label(messages, &choice.kind);
    if matches!(choice.kind.as_str(), "all" | "g-all" | "daily") {
        label.into()
    } else {
        format!("{label} {} {}", choice.id, choice.name)
    }
}

fn short_name(name: &str, limit: usize) -> String {
    let mut units = 0;
    name.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= limit
        })
        .collect()
}

pub(crate) fn render_page(selection: &Selection, messages: &Messages) -> String {
    let pages = selection
        .choices
        .len()
        .div_ceil(crate::commands::pagination::PAGE_SIZE);
    let lines = selection
        .choices
        .iter()
        .skip(selection.page * 10)
        .take(10)
        .enumerate()
        .map(|(index, c)| {
            format!(
                "{}. {} {} {}",
                index + 1,
                kind_label(messages, &c.kind),
                c.id,
                short_name(&c.name, 70)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    message!(
        messages,
        "push.choices",
        page = selection.page + 1,
        pages = pages,
        lines = lines,
        navigation = crate::commands::pagination::navigation(messages, selection.page, pages)
    )
}

pub(crate) async fn lookup(
    source: &crate::event_data::source::Source,
    messages: &Messages,
    args: &[String],
) -> Result<crate::event_data::Output> {
    let query = args[3..].join(" ").to_lowercase();
    let mut choices = Vec::new();
    if args[0] == "sale" {
        let data = source.sale().await?;
        let mut names = data.all_day_event_names;
        names.extend(data.sale_names);
        for (id, name) in names {
            if name.to_lowercase().contains(&query) {
                choices.push(Choice {
                    kind: "sale".into(),
                    id,
                    name: crate::skd::labels::strip_display_markup(&name, " "),
                });
            }
        }
    } else {
        let data = source.fetch_lookup_data().await?;
        for (kind, mode) in [
            ("gr", crate::skd::model::GachaMode::Rare),
            ("ge", crate::skd::model::GachaMode::Event),
            ("gn", crate::skd::model::GachaMode::Normal),
        ] {
            for (id, name) in data.gacha_names.get(mode) {
                if name.to_lowercase().contains(&query) {
                    choices.push(Choice {
                        kind: kind.into(),
                        id: *id,
                        name: crate::skd::labels::strip_display_markup(name, " "),
                    });
                }
            }
        }
    }
    for choice in &mut choices {
        choice.name = short_name(&choice.name, 24);
    }
    choices.sort_by(|a, b| (&a.kind, a.id).cmp(&(&b.kind, b.id)));
    if choices.is_empty() {
        return Ok(crate::event_data::Output::text(vec![message!(
            messages,
            "push.not_found",
            query = query
        )]));
    }
    if choices.len() > 512 {
        return Ok(crate::event_data::Output::text(vec![
            message!(messages, "push.too_many").into(),
        ]));
    }
    let selection = Selection {
        choices,
        page: 0,
        advance: args[1].parse()?,
        enabled: args[2].parse()?,
    };
    Ok(crate::event_data::Output {
        messages: vec![render_page(&selection, messages)],
        selection: Some(crate::event_data::Selection {
            kind: "push".into(),
            choices: Vec::new(),
            push: Some(selection),
        }),
    })
}

pub(crate) fn select(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    id: &str,
    selection: &mut Selection,
    pending: bool,
    messages: &Messages,
    now: i64,
) -> Result<Vec<Response>> {
    use crate::commands::pagination::{Input, PAGE_SIZE, parse};
    let CoreEvent::MessageReceived {
        text,
        event_id,
        chat_id,
        ..
    } = event
    else {
        return Ok(vec![]);
    };
    let input = parse(text, selection.page);
    if matches!(input, Input::Finish) {
        tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
        return Ok(vec![(
            message!(messages, "search.apply_inner_04").into(),
            now,
            None,
        )]);
    }
    if pending && matches!(input, Input::Select(_) | Input::Move(_)) {
        return Ok(vec![(
            message!(messages, "search.apply_inner_05").into(),
            now,
            None,
        )]);
    }
    let text = match input {
        Input::Move(page) => {
            let pages = selection.choices.len().div_ceil(PAGE_SIZE);
            let Some(page) = page.filter(|p| *p < pages) else {
                return Ok(vec![(
                    message!(messages, "navigation.invalid_page", pages = pages),
                    now,
                    None,
                )]);
            };
            if page == selection.page {
                return Ok(vec![]);
            }
            selection.page = page;
            let payload = serde_json::to_string(&crate::event_data::Selection {
                kind: "push".into(),
                choices: Vec::new(),
                push: Some(Selection {
                    choices: selection.choices.clone(),
                    page,
                    advance: selection.advance,
                    enabled: selection.enabled,
                }),
            })?;
            tx.execute(
                "UPDATE sessions SET action=?2,pending_payload=?3 WHERE id=?1",
                params![id, format!("{event_id}:0"), payload],
            )?;
            render_page(selection, messages)
        }
        Input::Select(number @ 1..=PAGE_SIZE) => {
            let Some(choice) = selection
                .choices
                .get(selection.page * PAGE_SIZE + number - 1)
            else {
                return Ok(vec![(
                    message!(messages, "search.apply_inner_07").into(),
                    now,
                    None,
                )]);
            };
            let text = configure(
                tx,
                chat_id,
                choice,
                selection.advance,
                selection.enabled,
                messages,
                now,
            )?;
            tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
            text
        }
        Input::Select(_) => message!(messages, "search.apply_inner_07").into(),
        _ => return Ok(vec![]),
    };
    Ok(vec![(text, now, None)])
}
