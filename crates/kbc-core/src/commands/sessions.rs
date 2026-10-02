use kbc_protocol::CoreEvent;
use rusqlite::{OptionalExtension, Transaction, params};

use super::{
    CommandPlan,
    search::{PAGE_SIZE, SESSION_TTL_MS, SearchCatalog, SearchSession, detail, image_url, label},
};
use crate::Result;

// 本文、配送期限、任意の画像URL。SDK固有の値はここへ渡さない。
pub type Response = (String, i64, Option<String>);

pub fn apply(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    plan: CommandPlan,
    catalog: &SearchCatalog,
    now: i64,
) -> Result<(Vec<Response>, Option<String>)> {
    let CoreEvent::MessageReceived {
        chat_id,
        sender_id,
        reply_to_message_id,
        ..
    } = event;
    let old: Option<String> = if let Some(owner) = sender_id {
        match &plan {
            CommandPlan::Search(_) => tx.query_row("SELECT prompt FROM sessions WHERE chat=?1 AND owner=?2", params![chat_id, owner], |row| row.get::<_, Option<String>>(0)).optional()?.flatten(),
            CommandPlan::Ignore if reply_to_message_id.is_some() => tx.query_row("SELECT prompt FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4", params![chat_id, owner, reply_to_message_id, now], |row| row.get::<_, Option<String>>(0)).optional()?.flatten(),
            _ => None,
        }
    } else {
        None
    };
    let responses = apply_inner(tx, event, plan, catalog, now)?;
    let replace = if !responses.is_empty()
        && let Some(prompt) = old
    {
        let still_active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3)",
            params![chat_id, sender_id, prompt],
            |row| row.get(0),
        )?;
        if still_active { None } else { Some(prompt) }
    } else {
        None
    };
    Ok((responses, replace))
}

fn apply_inner(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    plan: CommandPlan,
    catalog: &SearchCatalog,
    now: i64,
) -> Result<Vec<Response>> {
    let CoreEvent::MessageReceived {
        event_id,
        chat_id,
        message_id: _,
        text,
        sender_id,
        reply_to_message_id,
        ..
    } = event;
    match plan {
        CommandPlan::Text(messages) => Ok(messages
            .into_iter()
            .map(|(text, due)| (text, due, None))
            .collect()),
        CommandPlan::Search(session) => {
            if let Some(owner) = sender_id {
                tx.execute(
                    "DELETE FROM sessions WHERE chat=?1 AND owner=?2",
                    params![chat_id, owner],
                )?;
            }
            if session.query.is_empty() {
                return Ok(vec![("検索語を指定してください。".into(), now, None)]);
            }
            if session.results.is_empty() {
                return Ok(vec![(
                    format!("該当する{}が見つかりませんでした。", label(&session.kind)),
                    now,
                    None,
                )]);
            }
            if session.results.len() <= 3 {
                if session.origin && session.results.len() == 1 {
                    return Ok(selected(catalog, &session, 0, now));
                }
                if !session.origin {
                    return Ok(vec![(
                        session
                            .results
                            .iter()
                            .enumerate()
                            .map(|(index, _)| detail(catalog.entry(&session, index)))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        now,
                        None,
                    )]);
                }
            }
            let Some(owner) = sender_id else {
                return Ok(vec![(catalog.page(&session, false), now, None)]);
            };
            // 同じ人・同じトークの古い候補だけを入れ替える。
            let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))?;
            if count >= 128 {
                return Ok(vec![(
                    "検索の受付が混み合っています。IDで指定するか、少し待って再度お試しください。"
                        .into(),
                    now,
                    None,
                )]);
            }
            let page = catalog.page(&session, true);
            tx.execute("INSERT INTO sessions (id,chat,owner,action,payload,expires,revision) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![event_id, chat_id, owner, format!("{event_id}:0"), serde_json::to_string(&session)?, now + SESSION_TTL_MS, catalog.revision])?;
            Ok(vec![(page, now, None)])
        }
        CommandPlan::Ignore => {
            let (Some(owner), Some(prompt)) = (sender_id, reply_to_message_id) else {
                return Ok(Vec::new());
            };
            let stored: Option<(String, String)> = tx.query_row("SELECT id,payload FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4", params![chat_id, owner, prompt, now], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
            let Some((id, payload)) = stored else {
                return Ok(Vec::new());
            };
            let mut session: SearchSession = serde_json::from_str(&payload)?;
            let input = text.trim();
            if matches!(input, "終了" | "取消" | "cancel") {
                tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                return Ok(vec![("検索の受付を終了しました。".into(), now, None)]);
            }
            let page = match input {
                "9" | "次" if (session.page + 1) * PAGE_SIZE < session.results.len() => {
                    Some(session.page + 1)
                }
                "0" | "前" if session.page > 0 => Some(session.page - 1),
                _ => None,
            };
            if let Some(page) = page {
                session.page = page;
                tx.execute(
                    "UPDATE sessions SET prompt=NULL,action=?2,payload=?3,expires=?4 WHERE id=?1",
                    params![
                        id,
                        format!("{event_id}:0"),
                        serde_json::to_string(&session)?,
                        now + SESSION_TTL_MS
                    ],
                )?;
                return Ok(vec![(catalog.page(&session, true), now, None)]);
            }
            if let Ok(number @ 1..=8) = input.parse::<usize>()
                && input == number.to_string()
            {
                let index = session.page * PAGE_SIZE + number - 1;
                if index < session.results.len() {
                    tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                    return Ok(selected(catalog, &session, index, now));
                }
            }
            if (input.len() == 1 && input.bytes().all(|byte| byte.is_ascii_digit()))
                || matches!(input, "次" | "前")
            {
                return Ok(vec![("このページの候補番号をリプライしてください。9は次、0は前、終了で受付を終えます。".into(), now, None)]);
            }
            Ok(Vec::new())
        }
    }
}

fn selected(
    catalog: &SearchCatalog,
    session: &SearchSession,
    index: usize,
    now: i64,
) -> Vec<Response> {
    let entry = catalog.entry(session, index);
    let mut responses = vec![(detail(entry), now, None)];
    if session.origin {
        responses.push((String::new(), now, Some(image_url(session, entry))));
    }
    responses
}
