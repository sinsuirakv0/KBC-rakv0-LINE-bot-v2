//! 通常受信とは別のcursorでOC内の参加トークを辿り、本人の投稿だけを有限バッチで削除する。
use super::*;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Purge {
    chats: Vec<String>,
    index: usize,
    directory_token: Option<String>,
    directory_done: bool,
    listing_partial: bool,
    backward: bool,
    sync: Option<String>,
    continuation: Option<String>,
    pages: u32,
    scanned: u32,
    deleted: u32,
    pending: Vec<String>,
    batch_size: usize,
    in_flight: usize,
    last_ids: Vec<String>,
    stop: Option<String>,
    code: String,
    #[serde(default)]
    control_id: Option<String>,
    #[serde(default)]
    room_end: bool,
    #[serde(default)]
    joined_rooms: u32,
    #[serde(default)]
    history_rooms: u32,
}

pub(super) fn chat(job: &Job) -> Result<&str> {
    let state = job.purge.as_ref().ok_or("MissingPurge")?;
    state
        .chats
        .get(state.index)
        .map(String::as_str)
        .ok_or_else(|| "MissingPurgeChat".into())
}

pub(super) fn start(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    text: String,
    now: i64,
) -> Result<()> {
    let target = remote::chat(job).to_owned();
    let state = job.purge.as_mut().ok_or("MissingPurge")?;
    state.chats.push(target);
    state.batch_size = 20;
    state.control_id = Some(identity(&job.event).0.into());
    session(
        &runtime.content.messages,
        tx,
        job,
        Session::Purge { cancelled: false },
        message!(&runtime.content.messages, "purge.started", text = text),
        now,
    )?;
    request(
        tx,
        job,
        OcRequest::Inspect {
            chat_id: remote::chat(job).into(),
            member_ids: vec![],
        },
        Phase::MuteInspect,
        now,
    )
}

fn next(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    if cancelled(tx, job)? {
        return finish(runtime, tx, job, "purge.cancelled", now);
    }
    if job.purge.as_ref().ok_or("MissingPurge")?.room_end {
        end_room(job, true)?;
    }
    let state = job.purge.as_ref().ok_or("MissingPurge")?;
    if let Some(reason) = &state.stop {
        return finish(runtime, tx, job, reason, now);
    }
    if state.pages >= 1000 || now - identity(&job.event).4 >= 600_000 {
        return finish(runtime, tx, job, "purge.limit", now);
    }
    if !state.directory_done {
        return request(
            tx,
            job,
            OcRequest::JoinedChats {
                continuation_token: state.directory_token.clone(),
            },
            Phase::MuteChats,
            now,
        );
    }
    if state.index >= state.chats.len() {
        return finish(
            runtime,
            tx,
            job,
            if state.listing_partial {
                "purge.chat_limit"
            } else {
                "purge.finished"
            },
            now,
        );
    }
    let api = OcRequest::History {
        square_id: remote::context(job)?.square_id.clone(),
        backward: state.backward,
        sync_token: state.sync.clone(),
        continuation_token: state.continuation.clone(),
    };
    request(tx, job, api, Phase::MuteHistory, now)
}

fn delete_next(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    if cancelled(tx, job)? {
        return finish(runtime, tx, job, "purge.cancelled", now);
    }
    if now - identity(&job.event).4 >= 600_000 {
        return finish(runtime, tx, job, "purge.limit", now);
    }
    let square_id = remote::context(job)?.square_id.clone();
    let state = job.purge.as_mut().ok_or("MissingPurge")?;
    let message_ids = state
        .pending
        .iter()
        .take(state.batch_size)
        .cloned()
        .collect::<Vec<_>>();
    state.in_flight = message_ids.len();
    request(
        tx,
        job,
        OcRequest::DeleteMessages {
            square_id,
            message_ids,
        },
        Phase::MuteDelete,
        now,
    )
}

pub(super) fn complete(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    job.purge.as_mut().ok_or("MissingPurge")?.code = result.code.chars().take(64).collect();
    let stopped = cancelled(tx, job)?;
    if matches!(job.phase, Phase::MuteDelete) {
        let state = job.purge.as_mut().ok_or("MissingPurge")?;
        // 成否不明な通信を再実行しない。明確な引数拒否だけ同じ未削除候補を小分けする。
        if matches!(result.status, DeliveryStatus::Failed)
            && result.code == "ILLEGAL_ARGUMENT"
            && state.in_flight > 1
        {
            if stopped {
                return finish(runtime, tx, job, "purge.cancelled", now);
            }
            state.batch_size = state.in_flight / 2;
            return delete_next(runtime, tx, job, now);
        }
        if matches!(result.status, DeliveryStatus::Sent) {
            state.deleted += state.in_flight as u32;
            state.pending.drain(..state.in_flight);
            state.in_flight = 0;
            if stopped {
                return finish(runtime, tx, job, "purge.cancelled", now);
            }
            if !state.pending.is_empty() {
                return delete_next(runtime, tx, job, now);
            }
            return next(runtime, tx, job, now);
        }
    }
    if stopped {
        return finish(runtime, tx, job, "purge.cancelled", now);
    }
    if !matches!(result.status, DeliveryStatus::Sent) {
        let key = if matches!(result.status, DeliveryStatus::Unknown) {
            "purge.unknown"
        } else {
            "purge.failed"
        };
        return finish(runtime, tx, job, key, now);
    }
    let square = remote::context(job)?.square_id.clone();
    if matches!(job.phase, Phase::MuteInspect) {
        let Some(context) = result
            .oc_result
            .as_ref()
            .and_then(|value| value.context.as_ref())
        else {
            return finish(runtime, tx, job, "purge.failed", now);
        };
        if context.square_id != square
            || context.bot_member_id != remote::context(job)?.bot_member_id
            || context.actor.member_id != context.bot_member_id
            || context.actor.square_id != square
            || !matches!(context.actor.state.as_str(), "JOINED" | "2")
            || policy::role_rank(&context.bot_role) < 2
        {
            return finish(runtime, tx, job, "purge.no_authority", now);
        }
        return next(runtime, tx, job, now);
    }
    if matches!(job.phase, Phase::MuteChats) {
        let Some(response) = result.oc_result.as_ref() else {
            return finish(runtime, tx, job, "purge.failed", now);
        };
        if response.chats.len() > 30
            || response
                .continuation_token
                .as_ref()
                .is_some_and(|token| token.len() > 2048)
        {
            return finish(runtime, tx, job, "purge.failed", now);
        }
        let state = job.purge.as_mut().ok_or("MissingPurge")?;
        state.pages += 1;
        for entry in &response.chats {
            if entry.square_id != square || state.chats.contains(&entry.chat_id) {
                continue;
            }
            if state.chats.len() == 128 {
                state.listing_partial = true;
                continue;
            }
            state.chats.push(entry.chat_id.clone());
        }
        if let Some(token) = &response.continuation_token {
            if Some(token) == state.directory_token.as_ref() {
                state.listing_partial = true;
                state.directory_done = true;
            } else {
                state.directory_token = Some(token.clone());
            }
        } else {
            state.directory_done = true;
        }
        return next(runtime, tx, job, now);
    }
    let Some(page) = result
        .oc_result
        .as_ref()
        .and_then(|value| value.history.as_ref())
    else {
        return finish(runtime, tx, job, "purge.failed", now);
    };
    if page.event_count > 50
        || page.messages.len() > 50
        || page.joins.len() > 50
        || page.sync_token.len() > 2048
        || page
            .continuation_token
            .as_ref()
            .is_some_and(|token| token.len() > 2048)
    {
        return finish(runtime, tx, job, "purge.failed", now);
    }
    let target = job.targets.first().ok_or("MissingPurgeTarget")?.clone();
    let state = job.purge.as_mut().ok_or("MissingPurge")?;
    state.pages += 1;
    let advanced = state.sync.as_deref() != Some(&page.sync_token)
        || state.continuation != page.continuation_token;
    state.sync = Some(page.sync_token.clone());
    state.continuation = page.continuation_token.clone();
    if !state.backward {
        // 旧Botの履歴probeを参照し、まず現在位置を得てから過去方向へ切り替える。
        if page.event_count == 0 || !advanced {
            state.backward = true;
            state.continuation = None;
        }
        return next(runtime, tx, job, now);
    }
    if page.event_count == 0 {
        end_room(job, false)?;
        return next(runtime, tx, job, now);
    }
    let ids: Vec<String> = page
        .messages
        .iter()
        .map(|value| value.message_id.clone())
        .collect();
    if !ids.is_empty() && ids == state.last_ids {
        return finish(runtime, tx, job, "purge.stalled", now);
    }
    let previous_ids = std::mem::replace(&mut state.last_ids, ids);
    state.scanned += page.messages.len() as u32;
    let joins = page
        .joins
        .iter()
        .filter(|entry| entry.member_id == target)
        .collect::<Vec<_>>();
    let boundary = joins.iter().filter_map(|entry| entry.created_at_ms).max();
    state.room_end = !joins.is_empty();
    for value in &page.messages {
        if value.sender_id == target
            && (joins.is_empty()
                || boundary
                    .is_some_and(|at| value.created_at_ms.is_some_and(|created| created >= at)))
            && !previous_ids.contains(&value.message_id)
            && !state.pending.contains(&value.message_id)
        {
            state.pending.push(value.message_id.clone());
        }
    }
    if !advanced && !state.room_end {
        state.stop = Some("purge.stalled".into());
    }
    if state.pending.is_empty() {
        return next(runtime, tx, job, now);
    }
    delete_next(runtime, tx, job, now)
}

fn end_room(job: &mut Job, joined: bool) -> Result<()> {
    let state = job.purge.as_mut().ok_or("MissingPurge")?;
    if joined {
        state.joined_rooms += 1;
    } else {
        state.history_rooms += 1;
    }
    state.index += 1;
    state.backward = false;
    state.sync = None;
    state.continuation = None;
    state.last_ids.clear();
    state.room_end = false;
    Ok(())
}

fn cancelled(tx: &Transaction<'_>, job: &Job) -> Result<bool> {
    let Some(id) = job
        .purge
        .as_ref()
        .and_then(|state| state.control_id.as_ref())
    else {
        return Ok(false);
    };
    let payload: Option<String> = tx
        .query_row("SELECT payload FROM oc_sessions WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(match payload {
        Some(payload) => !matches!(
            serde_json::from_str::<Session>(&payload)?,
            Session::Purge { cancelled: false }
        ),
        None => true,
    })
}

pub(super) fn stop_reply(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    event: &CoreEvent,
    now: i64,
) -> Result<bool> {
    let CoreEvent::MessageReceived {
        text,
        reply_to_message_id: Some(prompt),
        ..
    } = event
    else {
        return Ok(false);
    };
    let (_, chat, _, owner, _) = identity(event);
    let stored: Option<(String, String)> = tx.query_row("SELECT id,payload FROM oc_sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4",
        params![chat,owner,prompt,now], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((id, payload)) = stored else {
        return Ok(false);
    };
    if !matches!(
        serde_json::from_str::<Session>(&payload)?,
        Session::Purge { .. }
    ) {
        return Ok(false);
    }
    let messages = &runtime.content.messages;
    if !matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "停止" | "終了" | "取消" | "stop" | "cancel"
    ) {
        text_action(
            tx,
            event,
            "purge-control",
            chat,
            message!(messages, "purge.stop_usage").into(),
            TextDelivery::default(),
            now,
        )?;
        return Ok(true);
    }
    tx.execute(
        "UPDATE oc_sessions SET payload=?2 WHERE id=?1",
        params![
            id,
            serde_json::to_string(&Session::Purge { cancelled: true })?
        ],
    )?;
    let pending: Option<(String,String,String)> = tx.query_row("SELECT id,payload,status FROM actions WHERE event_id=?1 AND status IN ('queued','querying','claimed','sending')
        AND json_extract(payload,'$.type')='ocApi' ORDER BY rowid DESC LIMIT 1", [&id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if let Some((action_id, payload, status)) = pending
        && status == "queued"
    {
        let CoreAction::OcApi { continuation, .. } = serde_json::from_str(&payload)? else {
            return Err("InvalidPurgeAction".into());
        };
        let mut job: Job = serde_json::from_str(&continuation)?;
        job.purge.as_mut().ok_or("MissingPurge")?.code = "UserStopped".into();
        tx.execute("UPDATE actions SET status='failed',code='PurgeCancelled',payload='',completed=?2 WHERE id=?1 AND status='queued'", params![action_id,now])?;
        finish(runtime, tx, &job, "purge.cancelled", now)?;
    } else {
        text_action(
            tx,
            event,
            "purge-control",
            chat,
            message!(messages, "purge.stop_requested").into(),
            TextDelivery::default(),
            now,
        )?;
    }
    Ok(true)
}

fn finish(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &Job,
    reason: &str,
    now: i64,
) -> Result<()> {
    let messages = &runtime.content.messages;
    let state = job.purge.as_ref().ok_or("MissingPurge")?;
    if let Some(id) = &state.control_id {
        tx.execute("DELETE FROM oc_sessions WHERE id=?1", [id])?;
    }
    history(
        messages,
        tx,
        job,
        job.targets.first().ok_or("MissingPurgeTarget")?,
        "purge",
        reason,
        now,
    )?;
    reply(
        tx,
        job,
        message!(
            messages,
            "purge.result",
            square = remote::context(job)?.square_id,
            rooms = state.chats.len(),
            completed = state.index,
            joined = state.joined_rooms,
            history = state.history_rooms,
            scanned = state.scanned,
            deleted = state.deleted,
            pending = state.in_flight,
            remaining = state.pending.len(),
            code = state.code,
            reason = messages.literal(reason),
        ),
        now,
    )
}
