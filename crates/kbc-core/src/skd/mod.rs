//! スケジュール差分を共有準備workerで生成し、親投稿からスレッド配送へ接続する。

use crate::{Result, Runtime, messages::message, now_ms};
use chrono::NaiveDate;
use kbc_protocol::{ActionResult, CoreAction, DeliveryStatus};
use rusqlite::{Connection, Transaction, params};
use std::sync::Arc;

mod diff;
mod formatter;
pub(crate) mod labels;
pub(crate) mod metadata;
pub(crate) mod model;
pub(crate) mod monitor;
mod parser;
mod source;
#[cfg(test)]
mod tests;

pub const MAX_THREAD_MESSAGES: usize = 32;
const THREAD_READY_DELAY_MS: i64 = 1000;

pub fn parse_date(args: &[&str]) -> Result<Option<String>> {
    if args.is_empty() {
        return Ok(None);
    }
    let value = args.join(" ");
    let parts: Vec<_> = value
        .split(|c: char| c == '/' || c.is_whitespace())
        .filter(|value| !value.is_empty())
        .collect();
    if parts.len() != 3 || parts[0].len() != 4 {
        return Err("InvalidScheduleDate".into());
    }
    let date = NaiveDate::from_ymd_opt(parts[0].parse()?, parts[1].parse()?, parts[2].parse()?)
        .filter(|_| parts[0].parse::<i32>().unwrap_or(0) >= 1000)
        .ok_or("InvalidScheduleDate")?;
    Ok(Some(date.to_string()))
}

impl Runtime {
    pub(crate) async fn prepare_schedule(&self, date: Option<String>) -> Result<Vec<String>> {
        let date = date
            .map(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d"))
            .transpose()?;
        let source = source::SkdDataSource::new(
            Arc::new(self.assets.clone()),
            Arc::clone(&self.content.messages),
        );
        let Some(update) = source.load(date).await? else {
            return Err("ScheduleUnavailable".into());
        };
        schedule_contents(self, update)
    }
}

fn schedule_contents(runtime: &Runtime, update: source::SkdUpdate) -> Result<Vec<String>> {
    let catalog = &runtime.content.messages;
    validate_contents(&[message!(catalog, "skd.root").into()])?;
    let mut contents = vec![message!(
        catalog,
        "skd.header",
        detected = crate::store_update::detected_at(update.timestamp.timestamp_millis())?,
        types = update
            .types
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(",")
    )];
    if !update.initial_types.is_empty() {
        contents.push(message!(
            catalog,
            "skd.initial",
            types = update
                .initial_types
                .iter()
                .map(|kind| kind.label())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    contents.extend(update.contents);
    validate_contents(&contents)?;
    Ok(contents)
}

fn validate_contents(contents: &[String]) -> Result<()> {
    if contents.is_empty()
        || contents.len() > MAX_THREAD_MESSAGES
        || contents.iter().any(|text| {
            text.is_empty() || text.encode_utf16().count() > 1500 || text.contains("```")
        })
    {
        return Err("ScheduleMessageLimit".into());
    }
    Ok(())
}

pub(crate) fn root_action(
    runtime: &Runtime,
    id: String,
    event: String,
    chat: String,
    contents: Vec<String>,
    now: i64,
) -> Result<CoreAction> {
    validate_contents(&contents)?;
    validate_contents(&[message!(runtime.content.messages, "skd.root").into()])?;
    Ok(CoreAction::SendMessage {
        action_id: id,
        event_id: event,
        chat_id: chat,
        related_message_id: String::new(),
        text: message!(runtime.content.messages, "skd.root").into(),
        thread_root_id: None,
        thread_contents: Some(contents),
        mention: None,
        emojis: None,
        image_url: None,
        attachment: None,
        replace_message_id: None,
        is_prompt: false,
        created_at_ms: now,
    })
}

pub(crate) fn pending_count(db: &Connection) -> Result<i64> {
    // 親の待機本文と準備jobにも配送枠を予約し、親の送信完了時に上限を超えない。
    Ok(db.query_row("SELECT COALESCE(sum(CASE WHEN NOT json_valid(payload) THEN 1 WHEN json_extract(payload,'$.type')='prepareMedia' THEN ?1 ELSE 1+COALESCE(json_array_length(payload,'$.threadContents'),0) END),0) FROM actions WHERE status IN ('queued','preparing','claimed','querying','sending','unknown')", [MAX_THREAD_MESSAGES as i64 + 1], |row| row.get(0))?)
}

pub(crate) fn complete(
    tx: &Transaction<'_>,
    action: &CoreAction,
    result: &ActionResult,
) -> Result<()> {
    if let CoreAction::SendMessage {
        event_id,
        chat_id,
        thread_root_id: Some(root),
        ..
    } = action
        && matches!(result.status, DeliveryStatus::Failed)
    {
        // 本文の確定失敗では後続も止め、照会失敗を各partで繰り返さない。
        tx.execute("UPDATE actions SET status='failed',code='PreviousThreadPartFailed',completed=?4,payload='' WHERE status='queued' AND event_id=?1 AND chat=?2 AND json_extract(payload,'$.threadRootId')=?3",params![event_id,chat_id,root,now_ms()])?;
    }
    let CoreAction::SendMessage {
        action_id,
        event_id,
        chat_id,
        thread_contents: Some(contents),
        ..
    } = action
    else {
        return Ok(());
    };
    if !matches!(result.status, DeliveryStatus::Sent) {
        return Ok(());
    }
    if event_id.starts_with("store:skd:")
        && !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform='skd' AND chat=?1)",
            [chat_id],
            |row| row.get::<_, bool>(0),
        )?
    {
        return Ok(());
    }
    let root = result
        .message_id
        .as_ref()
        .filter(|id| !id.is_empty())
        .ok_or("MissingThreadRootId")?;
    let now = now_ms();
    for (index, text) in contents.iter().enumerate() {
        let id = format!("{action_id}:thread:{index}");
        let part = CoreAction::SendMessage {
            action_id: id.clone(),
            event_id: event_id.clone(),
            chat_id: chat_id.clone(),
            related_message_id: String::new(),
            text: text.clone(),
            thread_root_id: Some(root.clone()),
            thread_contents: None,
            mention: None,
            emojis: None,
            image_url: None,
            attachment: None,
            replace_message_id: None,
            is_prompt: false,
            created_at_ms: now,
        };
        tx.execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,?5,?6,'queued')",params![id,event_id,chat_id,serde_json::to_string(&part)?,now+THREAD_READY_DELAY_MS,now])?;
    }
    Ok(())
}
