use kbc_protocol::{ActionResult, CoreAction, CoreEvent, DeliveryStatus, OcRequest};
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

fn page_step(session: &SearchSession, reaction: &str) -> Option<usize> {
    let count = session
        .files
        .as_ref()
        .map_or(session.results.len(), Vec::len);
    match reaction {
        "NICE" if (session.page + 1) * PAGE_SIZE < count => Some(session.page + 1),
        "LOVE" if session.page > 0 => Some(session.page - 1),
        _ => None,
    }
}

fn request_reaction(tx: &Transaction<'_>, event: &CoreEvent, now: i64) -> Result<()> {
    let CoreEvent::ReactionNotified {
        event_id,
        chat_id,
        message_id,
        reaction_type,
        created_at_ms,
    } = event
    else {
        return Ok(());
    };
    if *created_at_ms < now - 60_000 {
        return Ok(());
    }
    let stored: Option<(String, String, String)> = tx.query_row(
        "SELECT id,owner,payload FROM sessions WHERE chat=?1 AND prompt=?2 AND expires>?3 AND pending_payload IS NULL",
        params![chat_id, message_id, now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((id, owner, payload)) = stored else {
        return Ok(());
    };
    let session: SearchSession = serde_json::from_str(&payload)?;
    if page_step(&session, reaction_type).is_none() {
        return Ok(());
    }
    let action_id = format!("{event_id}:reaction");
    let action = CoreAction::OcApi {
        action_id: action_id.clone(),
        event_id: event_id.clone(),
        chat_id: chat_id.clone(),
        request: OcRequest::Reactions {
            message_id: message_id.clone(),
            member_id: owner,
            reaction_type: reaction_type.clone(),
        },
        continuation: id.clone(),
        created_at_ms: now,
    };
    tx.execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,?5,?5,'queued')",
        params![action_id, event_id, chat_id, serde_json::to_string(&action)?, now])?;
    // 照会とページ配送が終わるまで同じpromptの追加操作をまとめる。
    tx.execute(
        "UPDATE sessions SET action=?2,pending_payload=payload WHERE id=?1",
        params![id, action_id],
    )?;
    Ok(())
}

pub fn complete_reaction(
    tx: &Transaction<'_>,
    action: &CoreAction,
    result: &ActionResult,
    catalog: &SearchCatalog,
    now: i64,
) -> Result<(Vec<Response>, Option<String>)> {
    let CoreAction::OcApi {
        action_id,
        event_id,
        chat_id,
        request:
            OcRequest::Reactions {
                message_id,
                member_id,
                reaction_type,
            },
        continuation,
        ..
    } = action
    else {
        return Ok((vec![], None));
    };
    let stored: Option<(String, i64)> = tx.query_row(
        "SELECT payload,expires FROM sessions WHERE id=?1 AND action=?2 AND chat=?3 AND owner=?4 AND prompt=?5 AND expires>?6",
        params![continuation, action_id, chat_id, member_id, message_id, now], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let Some((payload, expires)) = stored else {
        return Ok((vec![], None));
    };
    tx.execute(
        "UPDATE sessions SET pending_payload=NULL WHERE id=?1",
        [continuation],
    )?;
    if !matches!(result.status, DeliveryStatus::Sent) {
        return Ok((vec![("リアクションを確認できませんでした。接続状態を確認し、外してから付け直してください。".into(), now, None)], None));
    }
    let Some(reaction) = result
        .oc_result
        .as_ref()
        .and_then(|value| value.reaction.as_ref())
    else {
        return Ok((vec![], None));
    };
    if reaction.member_id != *member_id
        || reaction.reaction_type != *reaction_type
        || reaction.updated_at_ms < expires - SESSION_TTL_MS - 30_000
        || reaction.updated_at_ms > now + 30_000
    {
        return Ok((vec![], None));
    }
    let mut session: SearchSession = serde_json::from_str(&payload)?;
    let Some(page) = page_step(&session, reaction_type) else {
        return Ok((vec![], None));
    };
    session.page = page;
    // 送信成功まで旧ページ・旧promptを残す。失敗後も番号が別ページを指さない。
    tx.execute(
        "UPDATE sessions SET action=?2,pending_payload=?3 WHERE id=?1",
        params![
            continuation,
            format!("{event_id}:0"),
            serde_json::to_string(&session)?
        ],
    )?;
    Ok((
        vec![(catalog.page(&session, true), now, None)],
        Some(message_id.clone()),
    ))
}

pub fn apply(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    plan: CommandPlan,
    catalog: &SearchCatalog,
    now: i64,
) -> Result<(Vec<Response>, Option<String>)> {
    if matches!(event, CoreEvent::ReactionNotified { .. }) {
        request_reaction(tx, event, now)?;
        return Ok((vec![], None));
    }
    let CoreEvent::MessageReceived {
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
            let stored: Option<(String, String, Option<String>)> = tx.query_row("SELECT id,payload,pending_payload FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4", params![chat_id, owner, prompt, now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
            let Some((id, payload, pending)) = stored else {
                return Ok(Vec::new());
            };
            let session: SearchSession = serde_json::from_str(&payload)?;
            let count = session
                .files
                .as_ref()
                .map_or(session.results.len(), Vec::len);
            let input = text.trim();
            if matches!(input, "終了" | "取消" | "cancel") {
                tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                return Ok(vec![("検索の受付を終了しました。".into(), now, None)]);
            }
            if pending.is_some()
                && input.bytes().all(|byte| byte.is_ascii_digit())
                && !input.is_empty()
            {
                return Ok(vec![("一覧を切り替えています。新しい一覧が届いてから、その一覧へ番号をリプライしてください。".into(), now, None)]);
            }
            if let Ok(number @ 1..=8) = input.parse::<usize>()
                && input == number.to_string()
            {
                let index = session.page * PAGE_SIZE + number - 1;
                if index < count {
                    tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                    return selected(tx, event, catalog, &session, index, now);
                }
            }
            if (input.len() == 1 && input.bytes().all(|byte| byte.is_ascii_digit()))
                || matches!(input, "次" | "前")
            {
                return Ok(vec![("項目選択はこの一覧への番号リプライです。ページ移動は一覧を長押しし、👍（いいね）で次・❤️（ハート）で前のリアクションを付けてください。終了で受付を終えます。".into(), now, None)]);
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
                .push_str("\nモーション生成を受け付けました。");
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
                    "ファイル選択には送信者情報が必要です。".into(),
                    now,
                    None,
                )]);
            };
            let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))?;
            if count >= 128 {
                return Ok(vec![(
                    "検索の受付が混み合っています。少し待って再度お試しください。".into(),
                    now,
                    None,
                )]);
            }
            let mut next = session.clone();
            next.files = Some(file_options(entry, &session.kind, form.as_deref()));
            next.results = vec![session.results[index].clone()];
            next.page = 0;
            tx.execute("INSERT INTO sessions (id,chat,owner,action,payload,expires,revision) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![event_id, chat_id, owner, format!("{event_id}:1"), serde_json::to_string(&next)?, now + SESSION_TTL_MS, catalog.revision])?;
            responses[0]
                .0
                .push_str("\n利用できるファイルを確認しています。");
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
            .push_str("\n指定した形態の素材はありません。");
    }
    Ok(responses)
}
