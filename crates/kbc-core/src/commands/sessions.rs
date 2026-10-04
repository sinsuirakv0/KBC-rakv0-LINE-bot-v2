use crate::messages::message;
use kbc_protocol::CoreEvent;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    CommandPlan,
    search::{
        Operation, PAGE_SIZE, SESSION_TTL_MS, SearchCatalog, SearchSession, file_options, label,
        motion_plan, origin_path,
    },
};
use crate::Result;

// 本文、配送期限、任意の素材要求。SDK固有の値はここへ渡さない。
pub type Response = (String, i64, Option<crate::media::MediaRequest>);

pub fn initialize(db: &Connection) -> Result<()> {
    let columns = db
        .prepare("PRAGMA table_info(sessions)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == "pending_payload") {
        db.execute_batch("ALTER TABLE sessions ADD COLUMN pending_payload TEXT")?;
    }
    Ok(())
}

pub fn apply(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    plan: CommandPlan,
    catalog: &SearchCatalog,
    now: i64,
) -> Result<(Vec<Response>, Option<String>)> {
    let CoreEvent::MessageReceived {
        event_id,
        chat_id,
        sender_id,
        reply_to_message_id,
        ..
    } = event
    else {
        return Ok((vec![], None));
    };
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
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND NOT(action=?4 AND pending_payload IS NOT NULL))",
            params![chat_id, sender_id, prompt, format!("{event_id}:0")],
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
    let message_catalog = catalog.messages();
    let CoreEvent::MessageReceived {
        event_id,
        chat_id,
        message_id: _,
        text,
        sender_id,
        reply_to_message_id,
        ..
    } = event
    else {
        return Ok(vec![]);
    };
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
                return Ok(vec![(
                    message!(message_catalog, "search.apply_inner_01").into(),
                    now,
                    None,
                )]);
            }
            if session.results.is_empty() {
                return Ok(vec![(
                    message!(
                        message_catalog,
                        "search.apply_inner_02",
                        arg0 = label(message_catalog, &session.kind)
                    ),
                    now,
                    None,
                )]);
            }
            if session.results.len() <= 3 {
                if !matches!(session.operation, Operation::Detail) && session.results.len() == 1 {
                    return selected(tx, event, catalog, &session, 0, now);
                }
                if matches!(session.operation, Operation::Detail) {
                    return Ok(vec![(
                        session
                            .results
                            .iter()
                            .enumerate()
                            .map(|(index, _)| catalog.detail(&session, index))
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
                    message!(message_catalog, "search.apply_inner_03").into(),
                    now,
                    None,
                )]);
            }
            let page = catalog.page(&session, true);
            tx.execute("INSERT INTO sessions (id,chat,owner,action,payload,expires,revision) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![event_id, chat_id, owner, format!("{event_id}:0"), serde_json::to_string(&session)?, now + SESSION_TTL_MS, catalog.session_revision()])?;
            Ok(vec![(page, now, None)])
        }
        CommandPlan::Ignore => {
            let (Some(owner), Some(prompt)) = (sender_id, reply_to_message_id) else {
                return Ok(Vec::new());
            };
            let stored: Option<(String, String, Option<String>)> = tx.query_row("SELECT id,payload,pending_payload FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4", params![chat_id, owner, prompt, now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
            let Some((id, payload, pending)) = stored else {
                return Ok(Vec::new());
            };
            let mut session: SearchSession = serde_json::from_str(&payload)?;
            let count = session
                .files
                .as_ref()
                .map_or(session.results.len(), Vec::len);
            let input = text.trim();
            if matches!(input, "終了" | "取消" | "cancel") {
                tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                return Ok(vec![(
                    message!(message_catalog, "search.apply_inner_04").into(),
                    now,
                    None,
                )]);
            }
            let is_number = !input.is_empty() && input.bytes().all(|byte| byte.is_ascii_digit());
            let is_page = matches!(input, "次" | "前")
                || input.strip_suffix('p').is_some_and(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                });
            if pending.is_some() && (is_number || is_page) {
                return Ok(vec![(
                    message!(message_catalog, "search.apply_inner_05").into(),
                    now,
                    None,
                )]);
            }
            if is_page {
                let pages = count.div_ceil(PAGE_SIZE);
                let page = match input {
                    "次" => session.page.checked_add(1),
                    "前" => session.page.checked_sub(1),
                    _ => input
                        .strip_suffix('p')
                        .and_then(|value| value.parse::<usize>().ok())
                        .and_then(|page| page.checked_sub(1)),
                };
                let Some(page) = page.filter(|page| *page < pages) else {
                    return Ok(vec![(
                        message!(message_catalog, "search.apply_inner_06", pages = pages),
                        now,
                        None,
                    )]);
                };
                if page == session.page {
                    return Ok(Vec::new());
                }
                session.page = page;
                // 送信成功まで旧ページを残し、確定失敗では同じ番号へ戻す。
                tx.execute(
                    "UPDATE sessions SET action=?2,pending_payload=?3 WHERE id=?1",
                    params![
                        id,
                        format!("{event_id}:0"),
                        serde_json::to_string(&session)?
                    ],
                )?;
                return Ok(vec![(catalog.page(&session, true), now, None)]);
            }
            if let Ok(number @ 1..=PAGE_SIZE) = input.parse::<usize>()
                && input == number.to_string()
            {
                let index = session.page * PAGE_SIZE + number - 1;
                if index < count {
                    tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                    return selected(tx, event, catalog, &session, index, now);
                }
            }
            if is_number {
                return Ok(vec![(
                    message!(message_catalog, "search.apply_inner_07").into(),
                    now,
                    None,
                )]);
            }
            Ok(Vec::new())
        }
    }
}

fn selected(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    catalog: &SearchCatalog,
    session: &SearchSession,
    index: usize,
    now: i64,
) -> Result<Vec<Response>> {
    let message_catalog = catalog.messages();
    use crate::media::MediaRequest;
    if let Some(files) = &session.files {
        let file = &files[index];
        return Ok(vec![
            (
                format!(
                    "{}\n{}\n{}",
                    catalog.entry_label(session, 0),
                    file.label,
                    catalog.asset_url(&file.path)?
                ),
                now,
                None,
            ),
            (
                String::new(),
                now,
                Some(MediaRequest::Download {
                    path: file.path.clone(),
                }),
            ),
        ]);
    }
    let entry = catalog.entry(session, index);
    let mut responses = vec![(catalog.detail(session, index), now, None)];
    let request = match &session.operation {
        Operation::Detail => return Ok(responses),
        Operation::Origin { family, form } => origin_path(entry, &session.kind, family, form)
            .map(|path| MediaRequest::Download { path }),
        Operation::Motion(request) => {
            responses[0]
                .0
                .push_str(message!(message_catalog, "search.selected_01"));
            motion_plan(entry, &session.kind, request)
                .map(|plan| MediaRequest::Motion(Box::new(plan)))
        }
        Operation::File { form } => {
            let CoreEvent::MessageReceived {
                event_id,
                chat_id,
                sender_id,
                ..
            } = event
            else {
                return Ok(vec![]);
            };
            let Some(owner) = sender_id else {
                return Ok(vec![(
                    message!(message_catalog, "search.selected_02").into(),
                    now,
                    None,
                )]);
            };
            let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))?;
            if count >= 128 {
                return Ok(vec![(
                    message!(message_catalog, "search.selected_03").into(),
                    now,
                    None,
                )]);
            }
            let mut next = session.clone();
            next.files = Some(file_options(
                message_catalog,
                entry,
                &session.kind,
                form.as_deref(),
            ));
            next.results = vec![session.results[index].clone()];
            next.page = 0;
            tx.execute("INSERT INTO sessions (id,chat,owner,action,payload,expires,revision) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![event_id, chat_id, owner, format!("{event_id}:1"), serde_json::to_string(&next)?, now + SESSION_TTL_MS, catalog.session_revision()])?;
            responses[0]
                .0
                .push_str(message!(message_catalog, "search.selected_04"));
            Some(MediaRequest::FileList {
                session_id: event_id.clone(),
            })
        }
    };
    if let Some(request) = request {
        responses.push((String::new(), now, Some(request)));
    } else {
        responses[0]
            .0
            .push_str(message!(message_catalog, "search.selected_05"));
    }
    Ok(responses)
}
