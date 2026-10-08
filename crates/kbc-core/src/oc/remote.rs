//! 実行元の本人・返信先と、遠隔OCの照会・設定先を分ける。
use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct Target {
    pub chat_id: String,
    pub context: Option<OcContext>,
}

pub(super) fn context(job: &Job) -> Result<&OcContext> {
    job.remote
        .as_ref()
        .and_then(|target| target.context.as_ref())
        .or(job.context.as_ref())
        .ok_or_else(|| "MissingOcContext".into())
}

pub(super) fn chat(job: &Job) -> &str {
    job.remote
        .as_ref()
        .map_or(identity(&job.event).1, |target| &target.chat_id)
}

pub(super) fn take_chat(input: &mut Input) -> std::result::Result<Option<String>, ()> {
    let mut chat = None;
    let mut args = Vec::new();
    for arg in &input.args {
        if let Some((key, value)) = arg.split_once(':')
            && key.eq_ignore_ascii_case("talkID")
        {
            if chat.is_some()
                || value.len() != 33
                || !value.starts_with('m')
                || !value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(());
            }
            chat = Some(value.to_owned());
        } else {
            args.push(arg.clone());
        }
    }
    input.args = args;
    Ok(chat)
}

pub(super) fn prepare(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    now: i64,
) -> Result<bool> {
    if !matches!(job.input.name.as_str(), "id" | "mute") {
        return Ok(false);
    }
    let messages = &runtime.content.messages;
    let target = match take_chat(&mut job.input) {
        Ok(Some(chat)) => chat,
        Ok(None) => return Ok(false),
        Err(()) => {
            reply(tx, job, message!(messages, "remote.invalid_chat"), now)?;
            return Ok(true);
        }
    };
    if job.input.name == "id"
        && (job.input.args.is_empty()
            || matches!(
                job.input.args[0].as_str(),
                "talk"
                    | "oc"
                    | "message"
                    | "msg"
                    | "reply"
                    | "metadata"
                    | "sticker"
                    | "stamp"
                    | "emoji"
                    | "me"
                    | "self"
                    | "log"
            ))
    {
        reply(tx, job, message!(messages, "remote.id_usage"), now)?;
        return Ok(true);
    }
    if target == identity(&job.event).1 {
        return Ok(false);
    }
    if bot_rank(tx, job)? < 2 {
        reply(tx, job, message!(messages, "remote.denied"), now)?;
        return Ok(true);
    }
    job.remote = Some(Target {
        chat_id: target.clone(),
        context: None,
    });
    request(
        tx,
        job,
        OcRequest::Inspect {
            chat_id: target,
            member_ids: vec![],
        },
        Phase::Remote,
        now,
    )?;
    Ok(true)
}

pub(super) fn complete(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let messages = &runtime.content.messages;
    if bot_rank(tx, job)? < 2 || identity(&job.event).4 < now - 60000 {
        return reply(tx, job, message!(messages, "remote.denied"), now);
    }
    if !matches!(result.status, DeliveryStatus::Sent) {
        return reply(
            tx,
            job,
            message!(messages, "remote.lookup_failed", code = result.code),
            now,
        );
    }
    let target = result
        .oc_result
        .as_ref()
        .and_then(|value| value.context.clone())
        .ok_or("MissingRemoteContext")?;
    if target.actor.member_id != target.bot_member_id
        || target.actor.square_id != target.square_id
        || !matches!(target.actor.state.as_str(), "JOINED" | "2")
    {
        return reply(
            tx,
            job,
            message!(messages, "remote.lookup_failed", code = "NotJoined"),
            now,
        );
    }
    job.remote.as_mut().ok_or("MissingRemoteTarget")?.context = Some(target);
    if job.input.name == "id" {
        id::execute(runtime, tx, job, now)
    } else {
        commands::execute(runtime, tx, job, now)
    }
}
