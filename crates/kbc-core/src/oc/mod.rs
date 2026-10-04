use crate::messages::message;
mod bot;
mod commands;
mod id;
mod legacy;
mod moderation;
mod policy;
mod test;
mod test_reply;
pub use legacy::import_legacy;

use crate::{Result, Runtime};
use kbc_protocol::*;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    url: bool,
    media: bool,
    left: bool,
    danger: bool,
    cohort: bool,
    report: bool,
    main: Option<String>,
    mod_room: Option<String>,
    source_chat: Option<String>,
    bot_member: Option<String>,
    rules: Vec<policy::UrlRule>,
    mutes: BTreeMap<String, Mute>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Mute {
    until: Option<i64>,
    name: String,
    since: i64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Template {
    text: String,
    mention: bool,
    show_id: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Notifications {
    join: Option<Template>,
    leave: Option<Template>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Input {
    name: String,
    args: Vec<String>,
    body: String,
}
#[derive(Serialize, Deserialize)]
enum Phase {
    Context,
    Target,
    Mutation,
    Report,
    Chats,
    Id,
    TestInspect,
}
#[derive(Serialize, Deserialize)]
struct Job {
    event: CoreEvent,
    input: Input,
    phase: Phase,
    step: u32,
    context: Option<OcContext>,
    targets: Vec<String>,
    results: Vec<String>,
    operation: String,
    case_id: Option<String>,
    deferred: Option<String>,
    #[serde(default)]
    id_lookup: Option<id::Lookup>,
    #[serde(default)]
    target_member: Option<OcMember>,
    #[serde(default)]
    test: Option<test::Plan>,
}
#[derive(Clone, Serialize, Deserialize)]
enum Session {
    Setup,
    Chats {
        chats: Vec<OcChat>,
        operation: String,
        template: Option<Template>,
        page: usize,
    },
}
pub fn initialize(db: &Connection) -> Result<()> {
    id::initialize(db)?;
    test::initialize(db)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS oc_settings(square TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS oc_notifications(chat TEXT PRIMARY KEY,square TEXT NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS oc_notifications_square ON oc_notifications(square);
        CREATE TABLE IF NOT EXISTS oc_sessions(id TEXT PRIMARY KEY,chat TEXT NOT NULL,owner TEXT NOT NULL,prompt TEXT,action TEXT NOT NULL,square TEXT NOT NULL,payload TEXT NOT NULL,expires INTEGER NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS oc_session_owner ON oc_sessions(chat,owner);
        CREATE TABLE IF NOT EXISTS oc_members(square TEXT NOT NULL,member TEXT NOT NULL,name TEXT NOT NULL,joined INTEGER NOT NULL,first INTEGER NOT NULL,visits INTEGER NOT NULL,state TEXT NOT NULL,last INTEGER NOT NULL,cohort INTEGER NOT NULL DEFAULT 0,suspicious INTEGER NOT NULL DEFAULT 0,messages INTEGER NOT NULL DEFAULT 0,last_text TEXT NOT NULL DEFAULT '',PRIMARY KEY(square,member));
        CREATE INDEX IF NOT EXISTS oc_members_last ON oc_members(last);
        CREATE TABLE IF NOT EXISTS oc_presence(square TEXT NOT NULL,chat TEXT NOT NULL,member TEXT NOT NULL,state TEXT NOT NULL,at INTEGER NOT NULL,name TEXT NOT NULL,PRIMARY KEY(square,chat,member));
        CREATE TABLE IF NOT EXISTS oc_media(square TEXT NOT NULL,member TEXT NOT NULL,message TEXT NOT NULL,chat TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(chat,message));
        CREATE INDEX IF NOT EXISTS oc_media_member ON oc_media(square,member,at);
        CREATE TABLE IF NOT EXISTS oc_notices(id TEXT PRIMARY KEY,at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS oc_cases(id TEXT PRIMARY KEY,square TEXT NOT NULL,chat TEXT NOT NULL,prompt TEXT,action TEXT NOT NULL,target TEXT NOT NULL,url TEXT,reason TEXT NOT NULL,state TEXT NOT NULL,expires INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS oc_history(id INTEGER PRIMARY KEY,square TEXT NOT NULL,target TEXT NOT NULL,actor TEXT NOT NULL,operation TEXT NOT NULL,status TEXT NOT NULL,detail TEXT NOT NULL,at INTEGER NOT NULL,action TEXT NOT NULL DEFAULT '');
        CREATE UNIQUE INDEX IF NOT EXISTS oc_history_action ON oc_history(action) WHERE action<>'';")?;
    let columns = db
        .prepare("PRAGMA table_info(oc_history)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for name in ["target_name", "actor_name", "reason"] {
        if !columns.iter().any(|column| column == name) {
            db.execute_batch(&format!(
                "ALTER TABLE oc_history ADD COLUMN {name} TEXT NOT NULL DEFAULT ''"
            ))?;
        }
    }
    Ok(())
}
fn settings(db: &Connection, square: &str) -> Result<Settings> {
    db.query_row(
        "SELECT payload FROM oc_settings WHERE square=?1",
        [square],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map_or(Ok(Settings::default()), |text| {
        Ok(serde_json::from_str(&text)?)
    })
}
fn save_settings(db: &Connection, square: &str, value: &Settings) -> Result<()> {
    let payload = serde_json::to_string(value)?;
    if payload.len() > 192 * 1024 || value.rules.len() > 100 || value.mutes.len() > 100 {
        return Err("OcSettingLimit".into());
    }
    db.execute("INSERT INTO oc_settings VALUES(?1,?2) ON CONFLICT(square) DO UPDATE SET payload=excluded.payload",params![square,payload])?;
    if db.query_row("SELECT count(*) FROM oc_settings", [], |r| {
        r.get::<_, i64>(0)
    })? > 2048
    {
        return Err("OcSettingLimit".into());
    }
    Ok(())
}
fn notifications(db: &Connection, chat: &str) -> Result<Notifications> {
    db.query_row(
        "SELECT payload FROM oc_notifications WHERE chat=?1",
        [chat],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map_or(Ok(Notifications::default()), |text| {
        Ok(serde_json::from_str(&text)?)
    })
}
pub fn priority_chats(db: &Connection) -> Result<Vec<String>> {
    // 通知設定済みトークと監視対象の本OCだけを高頻度取得の候補にする。
    let mut statement = db.prepare("SELECT chat FROM oc_notifications WHERE json_extract(payload,'$.join') IS NOT NULL OR json_extract(payload,'$.leave') IS NOT NULL
        UNION SELECT json_extract(payload,'$.main') FROM oc_settings WHERE json_extract(payload,'$.left')=1 OR json_extract(payload,'$.danger')=1 OR json_extract(payload,'$.cohort')=1 LIMIT 2049")?;
    let chats = statement
        .query_map([], |row| row.get::<_, Option<String>>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if chats.len() > 2048 {
        return Err("PriorityChatCapacity".into());
    }
    Ok(chats)
}
pub(super) fn identity(event: &CoreEvent) -> (&str, &str, &str, &str, i64) {
    match event {
        CoreEvent::ReactionNotified {
            event_id,
            chat_id,
            message_id,
            created_at_ms,
            ..
        } => (event_id, chat_id, message_id, "", *created_at_ms),
        CoreEvent::MessageReceived {
            event_id,
            chat_id,
            message_id,
            sender_id,
            created_at_ms,
            ..
        } => (
            event_id,
            chat_id,
            message_id,
            sender_id.as_deref().unwrap_or(""),
            *created_at_ms,
        ),
        CoreEvent::MemberChanged {
            event_id,
            chat_id,
            member_id,
            created_at_ms,
            ..
        } => (event_id, chat_id, "", member_id, *created_at_ms),
    }
}
fn square(event: &CoreEvent) -> Option<&str> {
    match event {
        CoreEvent::ReactionNotified { .. } => None,
        CoreEvent::MessageReceived { square_id, .. } => square_id.as_deref(),
        CoreEvent::MemberChanged { square_id, .. } => Some(square_id),
    }
}
fn bot_rank(runtime: &Runtime, job: &Job) -> u8 {
    let (_, chat, _, actor, _) = identity(&job.event);
    let context = job.context.as_ref().expect("resolved context");
    runtime
        .permissions
        .rank(&context.square_id, actor)
        .max(runtime.permissions.rank(chat, actor))
}
fn allowed(runtime: &Runtime, job: &Job, bot: u8, oc: u8) -> bool {
    bot_rank(runtime, job) >= bot
        || (oc > 0
            && policy::role_rank(&job.context.as_ref().expect("resolved context").actor.role) >= oc)
}
#[derive(Default)]
struct TextDelivery {
    mention: Option<MessageMention>,
    replace: Option<String>,
    prompt: bool,
    related_message_id: Option<String>,
}

// 文面の編集やサロゲートペアを含む名前でも、送信本文のUTF-16位置を使う。
fn mention_span(text: &str, label: &str, member_id: &str) -> Option<MessageMention> {
    let offset = text.find(label)?;
    let start = text[..offset].encode_utf16().count() as u32;
    Some(MessageMention {
        member_id: member_id.into(),
        start,
        end: start + label.encode_utf16().count() as u32,
    })
}
fn text_action(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    suffix: &str,
    chat: &str,
    text: String,
    delivery: TextDelivery,
    now: i64,
) -> Result<String> {
    let TextDelivery {
        mention,
        replace,
        prompt,
        related_message_id,
    } = delivery;
    let (root, _, _, _, _) = identity(event);
    let responses = crate::commands::split_responses(vec![(text, now, None)])?;
    let mut first = String::new();
    for (index, (text, _, _)) in responses.into_iter().enumerate() {
        let id = format!("{root}:oc:{suffix}:{index}");
        if index == 0 {
            first = id.clone();
        }
        let action = CoreAction::SendMessage {
            action_id: id.clone(),
            event_id: root.into(),
            chat_id: chat.into(),
            related_message_id: related_message_id.clone().unwrap_or_default(),
            emojis: None,
            text,
            image_url: None,
            attachment: None,
            mention: if index == 0 {
                mention.as_ref().map(|value| MessageMention {
                    member_id: value.member_id.clone(),
                    start: value.start,
                    end: value.end,
                })
            } else {
                None
            },
            replace_message_id: if index == 0 { replace.clone() } else { None },
            is_prompt: prompt && index == 0,
            created_at_ms: now,
        };
        insert_action(tx, &action, now)?;
    }
    Ok(first)
}
fn reply(tx: &Transaction<'_>, job: &Job, text: impl Into<String>, now: i64) -> Result<()> {
    text_action(
        tx,
        &job.event,
        &format!("reply-{}", job.step),
        identity(&job.event).1,
        text.into(),
        TextDelivery::default(),
        now,
    )?;
    Ok(())
}
fn insert_action(db: &Connection, action: &CoreAction, now: i64) -> Result<()> {
    let (id, event, chat) = match action {
        CoreAction::SendMessage {
            action_id,
            event_id,
            chat_id,
            ..
        }
        | CoreAction::DeleteMessage {
            action_id,
            event_id,
            chat_id,
            ..
        }
        | CoreAction::OcApi {
            action_id,
            event_id,
            chat_id,
            ..
        }
        | CoreAction::PrepareMedia {
            action_id,
            event_id,
            chat_id,
            ..
        } => (action_id, event_id, chat_id),
    };
    db.execute("INSERT OR IGNORE INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,?5,?5,'queued')",params![id,event,chat,serde_json::to_string(action)?,now])?;
    if db.query_row("SELECT count(*) FROM actions WHERE status IN ('queued','preparing','claimed','querying','sending','unknown')",[],|r|r.get::<_,i64>(0))?>2048 { return Err("ActionCapacity".into()); }
    Ok(())
}
fn request(
    tx: &Transaction<'_>,
    job: &mut Job,
    request: OcRequest,
    phase: Phase,
    now: i64,
) -> Result<()> {
    let read = request.is_read();
    job.phase = phase;
    job.step += 1;
    let (event, chat, _, _, _) = identity(&job.event);
    let action = CoreAction::OcApi {
        action_id: format!("{event}:oc:api:{}", job.step),
        event_id: event.into(),
        chat_id: chat.into(),
        request,
        continuation: serde_json::to_string(job)?,
        created_at_ms: now,
    };
    insert_action(tx, &action, now)?;
    if read {
        tx.execute(
            "UPDATE actions SET due=?2 WHERE id=?1",
            params![
                format!("{event}:oc:api:{}", job.step),
                identity(&job.event).4.min(now)
            ],
        )?;
    }
    Ok(())
}
fn history(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &Job,
    target: &str,
    status: &str,
    detail: &str,
    now: i64,
) -> Result<()> {
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    let target_name = job
        .target_member
        .as_ref()
        .filter(|member| member.member_id == target)
        .or_else(|| (context.actor.member_id == target).then_some(&context.actor))
        .map_or("", |member| member.name.as_str());
    let automatic = job.input.name == "moderate" || job.input.name == "signal";
    let actor_name = if automatic {
        "bot"
    } else {
        context.actor.name.as_str()
    };
    let actor_id = if automatic {
        if context.bot_member_id.is_empty() {
            "system"
        } else {
            &context.bot_member_id
        }
    } else {
        identity(&job.event).3
    };
    tx.execute("INSERT INTO oc_history(square,target,actor,operation,status,detail,at,target_name,actor_name,reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![context.square_id,target,actor_id,job.operation,status,detail.chars().take(300).collect::<String>(),now,target_name,actor_name,commands::reason(message_catalog, job)])?;
    tx.execute("DELETE FROM oc_history WHERE id IN (SELECT id FROM oc_history ORDER BY id DESC LIMIT -1 OFFSET 2048)",[])?;
    Ok(())
}
pub fn ingest(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    event: &CoreEvent,
    plan: &crate::commands::CommandPlan,
    now: i64,
) -> Result<bool> {
    tx.execute("DELETE FROM oc_sessions WHERE expires<=?1", [now])?;
    if matches!(event, CoreEvent::ReactionNotified { .. }) {
        return Ok(false);
    }
    if matches!(event, CoreEvent::MemberChanged { .. }) {
        moderation::member_event(runtime, tx, event, now)?;
        return Ok(true);
    }
    let CoreEvent::MessageReceived {
        sender_id,
        bot_member_id,
        text,
        reply_to_message_id,
        ..
    } = event
    else {
        unreachable!()
    };
    let Some(actor) = sender_id else {
        return Ok(false);
    };
    id::remember(tx, event)?;
    if bot_member_id.as_ref() == Some(actor) {
        return Ok(true);
    }
    if moderation::mute(runtime, tx, event, now)? {
        return Ok(true);
    }
    let input = if let Some(input) = commands::parse(text)
        .or_else(|| bot::parse(text))
        .or_else(|| id::parse(text))
        .or_else(|| test_reply::parse(text))
        .or_else(|| test::parse(text))
    {
        Some(input)
    } else if let Some(prompt) = reply_to_message_id {
        let session:Option<String>=tx.query_row("SELECT id FROM oc_sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4",params![identity(event).1,actor,prompt,now],|r|r.get(0)).optional()?;
        let case:Option<String>=tx.query_row("SELECT id FROM oc_cases WHERE chat=?1 AND prompt=?2 AND state='open' AND expires>?3",params![identity(event).1,prompt,now],|r|r.get(0)).optional()?;
        session
            .map(|id| Input {
                name: "session".into(),
                args: vec![id],
                body: text.trim().into(),
            })
            .or_else(|| {
                case.map(|id| Input {
                    name: "case".into(),
                    args: vec![id],
                    body: text.trim().into(),
                })
            })
    } else {
        None
    };
    if let Some(input) = input {
        if !actor.starts_with('p') || text.len() > 8192 {
            return Ok(true);
        }
        let mut job = Job {
            event: event.clone(),
            input,
            phase: Phase::Context,
            step: 0,
            context: None,
            targets: vec![],
            results: vec![],
            operation: "command".into(),
            case_id: None,
            deferred: None,
            id_lookup: None,
            target_member: None,
            test: None,
        };
        let authority = job.input.name == "authority";
        request(
            tx,
            &mut job,
            OcRequest::Context {
                member_id: actor.clone(),
                authority,
            },
            Phase::Context,
            now,
        )?;
        return Ok(true);
    }
    if let Some(reason) = moderation::candidate(tx, event, plan, now)? {
        let mut job = Job {
            event: event.clone(),
            input: Input {
                name: "moderate".into(),
                args: vec![reason],
                body: String::new(),
            },
            phase: Phase::Context,
            step: 0,
            context: None,
            targets: vec![],
            results: vec![],
            operation: "moderate".into(),
            case_id: None,
            deferred: Some(serde_json::to_string(plan)?),
            id_lookup: None,
            target_member: None,
            test: None,
        };
        request(
            tx,
            &mut job,
            OcRequest::Context {
                member_id: actor.clone(),
                authority: false,
            },
            Phase::Context,
            now,
        )?;
        return Ok(true);
    }
    Ok(false)
}
pub fn complete(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    action: &CoreAction,
    result: &ActionResult,
    resolving: bool,
    now: i64,
) -> Result<()> {
    let message_catalog = &runtime.content.messages;
    let CoreAction::OcApi {
        continuation,
        request: api,
        ..
    } = action
    else {
        return sent_prompt(tx, action, result, now);
    };
    if continuation.len() > 48 * 1024
        || result
            .oc_result
            .as_ref()
            .is_some_and(|value| serde_json::to_vec(value).map_or(true, |v| v.len() > 32 * 1024))
    {
        return Err("OcResultLimit".into());
    }
    if matches!(api, OcRequest::Reactions { .. }) {
        // 旧snapshotの照会は完了させるが、ページ操作には使わない。
        return Ok(());
    }
    let mut job: Job = serde_json::from_str(continuation)?;
    if job.input.name == "test" && matches!(job.phase, Phase::Mutation) {
        return test::mutated(message_catalog, tx, &job, action, result, resolving, now);
    }
    if matches!(job.phase, Phase::TestInspect) {
        return test::inspected(runtime, tx, &mut job, result, now);
    }
    if resolving {
        history(
            message_catalog,
            tx,
            &job,
            job.targets.first().map(String::as_str).unwrap_or(""),
            "resolved",
            &result.code,
            now,
        )?;
        return Ok(());
    }
    if api.is_read() && !matches!(result.status, DeliveryStatus::Sent) {
        if matches!(job.phase, Phase::Target) && !job.targets.is_empty() {
            let target = job.targets.remove(0);
            job.results.push(message!(
                message_catalog,
                "mod.complete_01",
                target = target
            ));
            return commands::next_target(message_catalog, tx, &mut job, now);
        }
        if job.input.name != "moderate" {
            reply(tx, &job, message!(message_catalog, "mod.complete_02"), now)?;
        }
        return Ok(());
    }
    match job.phase {
        Phase::Context => {
            let context = result
                .oc_result
                .as_ref()
                .and_then(|value| value.context.clone())
                .ok_or("MissingOcContext")?;
            if context.actor.member_id != identity(&job.event).3
                || context.actor.square_id != context.square_id
                || square(&job.event).is_some_and(|square| square != context.square_id)
                || !matches!(context.actor.state.as_str(), "JOINED" | "2")
            {
                return reply(tx, &job, message!(message_catalog, "mod.complete_03"), now);
            }
            job.context = Some(context);
            if identity(&job.event).4 < now - 60000 {
                return reply(tx, &job, message!(message_catalog, "mod.complete_04"), now);
            }
            if job.input.name == "bot-name" {
                bot::execute(runtime, tx, &mut job, now)
            } else if job.input.name == "id" {
                id::execute(runtime, tx, &mut job, now)
            } else if job.input.name == "test-reply" {
                test_reply::execute(runtime, tx, &job, now)
            } else if job.input.name == "test" {
                test::execute(runtime, tx, &mut job, now)
            } else if job.input.name == "moderate" {
                moderation::execute(runtime, tx, &mut job, now)
            } else {
                commands::execute(runtime, tx, &mut job, now)
            }
        }
        Phase::Target => {
            if job.input.name != "signal" && identity(&job.event).4 < now - 60000 {
                return reply(tx, &job, message!(message_catalog, "mod.complete_05"), now);
            }
            if job.input.name == "bot-name" {
                bot::target(runtime, tx, &mut job, result, now)
            } else {
                commands::target(runtime, tx, &mut job, result, now)
            }
        }
        Phase::Mutation if job.input.name == "bot-name" => {
            bot::mutation(message_catalog, tx, &job, result, now)
        }
        Phase::Mutation => commands::mutation(message_catalog, tx, &mut job, result, now),
        Phase::Chats => commands::chats(message_catalog, tx, &mut job, result, now),
        Phase::Report => moderation::after_report(message_catalog, tx, &mut job, result, now),
        Phase::Id => id::complete(message_catalog, tx, &mut job, result, now),
        Phase::TestInspect => unreachable!(),
    }
}
fn sent_prompt(
    tx: &Transaction<'_>,
    action: &CoreAction,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    if let CoreAction::DeleteMessage { action_id, .. } = action {
        tx.execute(
            "UPDATE oc_history SET status=?2,detail=detail || ' / ' || ?3 WHERE action=?1",
            params![
                action_id,
                match result.status {
                    DeliveryStatus::Sent => "成功",
                    DeliveryStatus::Failed => "失敗",
                    DeliveryStatus::Unknown => "結果不明",
                },
                result.code
            ],
        )?;
    }
    match action {
        CoreAction::SendMessage {
            action_id,
            event_id,
            chat_id,
            ..
        } if matches!(result.status, DeliveryStatus::Sent) => {
            if action_id.ends_with(":oc:mute-notice:0")
                && let Some(message_id) = &result.message_id
            {
                let id = format!("{action_id}:cleanup");
                insert_action(
                    tx,
                    &CoreAction::DeleteMessage {
                        action_id: id.clone(),
                        event_id: event_id.clone(),
                        chat_id: chat_id.clone(),
                        message_id: message_id.clone(),
                        created_at_ms: now,
                    },
                    now,
                )?;
                tx.execute(
                    "UPDATE actions SET due=?2 WHERE id=?1",
                    params![id, now + 15000],
                )?;
            }
            let waiting: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM oc_sessions WHERE action=?1 AND expires>?2)",
                params![action_id, now],
                |r| r.get(0),
            )?;
            if waiting && result.message_id.is_none() {
                return Err("MissingOcPromptId".into());
            }
            tx.execute(
                "UPDATE oc_sessions SET prompt=?2,expires=?3 WHERE action=?1",
                params![action_id, result.message_id, now + 600000],
            )?;
            tx.execute(
                "UPDATE oc_cases SET prompt=?2 WHERE action=?1",
                params![action_id, result.message_id],
            )?;
        }
        CoreAction::SendMessage { action_id, .. }
            if matches!(result.status, DeliveryStatus::Failed) =>
        {
            tx.execute("DELETE FROM oc_sessions WHERE action=?1", [action_id])?;
        }
        CoreAction::DeleteMessage {
            chat_id,
            message_id,
            ..
        } if matches!(result.status, DeliveryStatus::Sent) => {
            tx.execute(
                "DELETE FROM oc_sessions WHERE chat=?1 AND prompt=?2",
                params![chat_id, message_id],
            )?;
        }
        _ => {}
    };
    Ok(())
}
fn session(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &Job,
    value: Session,
    text: String,
    now: i64,
) -> Result<()> {
    let (id, chat, _, owner, _) = identity(&job.event);
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM oc_sessions WHERE chat=?1 AND owner=?2)",
        params![chat, owner],
        |r| r.get(0),
    )?;
    if !existing
        && tx.query_row("SELECT count(*) FROM oc_sessions", [], |r| {
            r.get::<_, i64>(0)
        })? >= 128
    {
        return reply(tx, job, message!(message_catalog, "mod.session_01"), now);
    }
    let old: Option<String> = tx
        .query_row(
            "SELECT prompt FROM oc_sessions WHERE chat=?1 AND owner=?2",
            params![chat, owner],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    tx.execute(
        "DELETE FROM oc_sessions WHERE chat=?1 AND owner=?2",
        params![chat, owner],
    )?;
    let action = text_action(
        tx,
        &job.event,
        &format!("prompt-{}", job.step),
        chat,
        text,
        TextDelivery {
            replace: old,
            prompt: true,
            ..Default::default()
        },
        now,
    )?;
    tx.execute(
        "INSERT INTO oc_sessions VALUES(?1,?2,?3,NULL,?4,?5,?6,?7)",
        params![
            id,
            chat,
            owner,
            action,
            job.context.as_ref().ok_or("MissingOcContext")?.square_id,
            serde_json::to_string(&value)?,
            now + 600000
        ],
    )?;
    if tx.query_row("SELECT count(*) FROM oc_sessions", [], |r| {
        r.get::<_, i64>(0)
    })? > 128
    {
        return Err("OcSessionLimit".into());
    }
    Ok(())
}
