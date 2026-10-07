//! イベント参照Commandを既存準備Worker・Outbox・Sessionへ載せる。
use crate::{
    Result, Runtime,
    messages::{Messages, message},
    now_ms,
};
use kbc_protocol::CoreAction;
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
mod gatya;
mod item;
mod sale;
mod source;
#[cfg(test)]
mod tests;
use source::Source;

fn format_time_block(block: &crate::skd::model::TimeBlock, messages: &Messages) -> String {
    let day = if !block.weekdays.is_empty() {
        let days = block
            .weekdays
            .iter()
            .map(|value| match value.as_str() {
                "Sun" => "日",
                "Mon" => "月",
                "Tue" => "火",
                "Wed" => "水",
                "Thu" => "木",
                "Fri" => "金",
                "Sat" => "土",
                value => value,
            })
            .collect::<Vec<_>>()
            .join("・");
        message!(messages, "event.weekly", days = days)
    } else if !block.month_days.is_empty() {
        message!(
            messages,
            "event.monthly",
            days = block
                .month_days
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    } else if !block.date_ranges.is_empty() {
        block
            .date_ranges
            .iter()
            .map(|range| {
                format!(
                    "{}~{}",
                    format_date_range_point(&range.start),
                    format_date_range_point(&range.end)
                )
            })
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        message!(messages, "event.daily").into()
    };
    let time = if block.time_ranges.is_empty() {
        message!(messages, "event.all_day").into()
    } else {
        block
            .time_ranges
            .iter()
            .map(|range| {
                format!(
                    "{}~{}",
                    format_minutes(parse_time_minutes(&range[0])),
                    format_minutes(parse_time_minutes(&range[1]))
                )
            })
            .collect::<Vec<_>>()
            .join("、")
    };
    format!("{day}  {time}")
}

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub command: String,
    pub arguments: Vec<String>,
    pub owner: Option<String>,
}
pub(crate) struct Output {
    pub messages: Vec<String>,
    pub selection: Option<Selection>,
}
impl Output {
    fn text(messages: Vec<String>) -> Self {
        Self {
            messages,
            selection: None,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Selection {
    pub kind: String,
    pub choices: Vec<i64>,
}

// 通常返信と同じ1,500 UTF-16単位で分割し、Discord版と同じ最大32件に制限する。
fn push_message(output: &mut Vec<String>, content: impl Into<String>) -> Result<()> {
    let content = content.into();
    if content.trim().is_empty() {
        return Err("EmptyEventResponse".into());
    }
    output.extend(crate::commands::split_text(&content));
    if output.len() > 32 {
        return Err("EventResponseLimit".into());
    }
    Ok(())
}

impl Runtime {
    pub(crate) async fn prepare_event_data(&self, request: &Request) -> Result<Output> {
        let source = Source::new(
            Arc::new(self.assets.clone()),
            Arc::clone(&self.content.messages),
        );
        let messages = &self.content.messages;
        let now = chrono::DateTime::from_timestamp_millis(now_ms()).ok_or("InvalidEventTime")?;
        let output = match request.command.as_str() {
            "gatya" => gatya::run(&source, messages, &request.arguments, now)
                .await
                .map(Output::text),
            "sale" => {
                sale::run(
                    &source,
                    messages,
                    &request.arguments,
                    now,
                    request.owner.is_some(),
                )
                .await
            }
            "item" => item::run(&source, messages, &request.arguments, now)
                .await
                .map(Output::text),
            _ => Err("InvalidEventCommand".into()),
        };
        match output {
            Ok(output) => Ok(output),
            Err(error) => {
                eprintln!("Event data preparation failed: {error}");
                Ok(Output::text(vec![
                    message!(messages, "event.failed").into(),
                ]))
            }
        }
    }
    pub(crate) fn finish_event_data(
        &self,
        action: CoreAction,
        request: Request,
        mut output: Output,
    ) -> Result<()> {
        let CoreAction::PrepareMedia {
            action_id,
            event_id,
            chat_id,
            related_message_id,
            replace_message_id,
            created_at_ms,
            ..
        } = action
        else {
            return Err("InvalidEventAction".into());
        };
        if output.messages.is_empty() || output.messages.len() > 32 {
            return Err("EventResponseLimit".into());
        }
        let now = now_ms();
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        let mut is_prompt = false;
        let wanted_selection = output.selection.is_some();
        if let (Some(owner), Some(selection)) = (request.owner, output.selection.take()) {
            let newer: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN actions a ON a.id=s.action WHERE s.chat=?1 AND s.owner=?2 AND a.created>?3)", params![chat_id, owner, created_at_ms], |r| r.get(0))?;
            if !newer {
                tx.execute(
                    "DELETE FROM sessions WHERE chat=?1 AND owner=?2",
                    params![chat_id, owner],
                )?;
                let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
                let payload = serde_json::to_string(&selection)?;
                if count < 128 && payload.len() <= 64 * 1024 && output.messages.len() == 1 {
                    tx.execute("INSERT INTO sessions(id,chat,owner,action,payload,expires,revision) VALUES(?1,?2,?3,?4,?5,?6,'event-v1')",
                        params![event_id, chat_id, owner, action_id, payload, now+30_000])?;
                    is_prompt = true;
                }
            }
        }
        if wanted_selection && !is_prompt {
            let text = output.messages[0]
                .strip_suffix(message!(self.content.messages, "event.select_hint"))
                .unwrap_or(&output.messages[0]);
            output.messages = crate::commands::split_text(&format!(
                "{text}\n\n{}",
                message!(self.content.messages, "event.selection_unavailable")
            ));
        }
        for (index, text) in output.messages.into_iter().enumerate() {
            let id = if index == 0 {
                action_id.clone()
            } else {
                format!("{action_id}:part:{index}")
            };
            let prepared = CoreAction::SendMessage {
                action_id: id.clone(),
                event_id: event_id.clone(),
                chat_id: chat_id.clone(),
                related_message_id: related_message_id.clone(),
                text,
                thread_root_id: None,
                thread_contents: None,
                mention: None,
                emojis: None,
                image_url: None,
                attachment: None,
                replace_message_id: if index == 0 {
                    replace_message_id.clone()
                } else {
                    None
                },
                is_prompt: index == 0 && is_prompt,
                created_at_ms,
            };
            let payload = serde_json::to_string(&prepared)?;
            if index == 0 {
                tx.execute("UPDATE actions SET status='queued',payload=?2,due=?3,code='' WHERE id=?1 AND status='preparing'", params![id,payload,now])?;
            } else {
                tx.execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,?5,?5,'queued')",params![id,event_id,chat_id,payload,now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn select(
    tx: &Transaction<'_>,
    id: &str,
    payload: &str,
    input: &str,
    messages: &Messages,
    now: i64,
) -> Result<Vec<crate::commands::sessions::Response>> {
    let selection: Selection = serde_json::from_str(payload)?;
    let response = match crate::commands::pagination::parse(input, 0) {
        crate::commands::pagination::Input::Select(number)
            if number > 0 && number <= selection.choices.len() =>
        {
            (
                String::new(),
                now,
                Some(crate::media::MediaJob {
                    catalog_revision: String::new(),
                    request: crate::media::MediaRequest::EventData(Request {
                        command: "sale".into(),
                        arguments: vec![selection.choices[number - 1].to_string()],
                        owner: None,
                    }),
                }),
            )
        }
        crate::commands::pagination::Input::Finish => (
            message!(messages, "search.apply_inner_04").into(),
            now,
            None,
        ),
        _ => {
            return Ok(vec![(
                message!(messages, "event.invalid_selection").into(),
                now,
                None,
            )]);
        }
    };
    tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
    Ok(vec![response])
}

fn parse_time_minutes(value: &str) -> i64 {
    let padded = format!("{:0>4}", value.trim());
    let hour = padded
        .get(0..2)
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    let minute = padded
        .get(2..4)
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    hour * 60 + minute
}

fn format_minutes(value: i64) -> String {
    if value >= 1_440 {
        return "24:00".to_owned();
    }
    format!("{:02}:{:02}", value / 60, value % 60)
}

fn format_date_range_point(value: &str) -> String {
    let mut parts = value.split_whitespace();
    let month_day = format!("{:0>4}", parts.next().unwrap_or("0"));
    let time = format!("{:0>4}", parts.next().unwrap_or("0"));
    let month = month_day
        .get(0..2)
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let day = month_day
        .get(2..4)
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let hour = time.get(0..2).unwrap_or("00");
    let minute = time.get(2..4).unwrap_or("00");
    format!("{month}/{day} {hour}:{minute}")
}
