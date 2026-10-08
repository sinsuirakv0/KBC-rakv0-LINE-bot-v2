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
}

pub(super) fn chat(job: &Job) -> Result<&str> {
    let state = job.purge.as_ref().ok_or("MissingPurge")?;
    state
        .chats
        .get(state.index)
        .map(String::as_str)
        .ok_or_else(|| "MissingPurgeChat".into())
}

pub(super) fn start(tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let target = remote::chat(job).to_owned();
    let state = job.purge.as_mut().ok_or("MissingPurge")?;
    state.chats.push(target);
    state.batch_size = 20;
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
    if matches!(job.phase, Phase::MuteDelete) {
        let state = job.purge.as_mut().ok_or("MissingPurge")?;
        // 成否不明な通信を再実行しない。明確な引数拒否だけ同じ未削除候補を小分けする。
        if matches!(result.status, DeliveryStatus::Failed)
            && result.code == "ILLEGAL_ARGUMENT"
            && state.in_flight > 1
        {
            state.batch_size = state.in_flight / 2;
            return delete_next(runtime, tx, job, now);
        }
        if matches!(result.status, DeliveryStatus::Sent) {
            state.deleted += state.in_flight as u32;
            state.pending.drain(..state.in_flight);
            state.in_flight = 0;
            if !state.pending.is_empty() {
                return delete_next(runtime, tx, job, now);
            }
            return next(runtime, tx, job, now);
        }
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
        state.index += 1;
        state.backward = false;
        state.sync = None;
        state.continuation = None;
        state.last_ids.clear();
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
    for value in &page.messages {
        if value.sender_id == target
            && !previous_ids.contains(&value.message_id)
            && !state.pending.contains(&value.message_id)
        {
            state.pending.push(value.message_id.clone());
        }
    }
    if !advanced {
        state.stop = Some("purge.stalled".into());
    }
    if state.pending.is_empty() {
        return next(runtime, tx, job, now);
    }
    delete_next(runtime, tx, job, now)
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
