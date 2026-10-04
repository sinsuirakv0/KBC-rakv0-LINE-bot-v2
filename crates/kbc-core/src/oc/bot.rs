use super::*;
use crate::messages::message;

pub fn parse(text: &str) -> Option<Input> {
    let mut input = text;
    let command = test_reply::take_word(&mut input);
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    if (!command.eq_ignore_ascii_case("!bot") && !command.eq_ignore_ascii_case("o.bot"))
        || !input[..end].eq_ignore_ascii_case("name")
    {
        return None;
    }
    // name直後の区切り用スペース1個だけを除き、残りの文字列はそのまま渡す。
    Some(Input {
        name: "bot-name".into(),
        args: vec![],
        body: input[end..]
            .strip_prefix(' ')
            .unwrap_or(&input[end..])
            .into(),
    })
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let message_catalog = &runtime.content.messages;
    if bot_rank(runtime, job) < 2 {
        return reply(
            tx,
            job,
            message!(message_catalog, "common.bot_admin_only"),
            now,
        );
    }
    let name = &job.input.body;
    if name.is_empty() {
        return reply(tx, job, message!(message_catalog, "bot.execute_01"), now);
    }
    let member_id = job
        .context
        .as_ref()
        .ok_or("MissingOcContext")?
        .bot_member_id
        .clone();
    request(tx, job, OcRequest::Member { member_id }, Phase::Target, now)
}

pub fn target(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let message_catalog = &runtime.content.messages;
    // 変更APIの発行時も、BOT管理者としての権限を確認する。
    if bot_rank(runtime, job) < 2 {
        return reply(
            tx,
            job,
            message!(message_catalog, "common.bot_admin_only"),
            now,
        );
    }
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    let Some(member) = result
        .oc_result
        .as_ref()
        .and_then(|value| value.member.clone())
    else {
        return reply(tx, job, message!(message_catalog, "bot.target_01"), now);
    };
    if member.member_id != context.bot_member_id
        || member.square_id != context.square_id
        || !matches!(member.state.as_str(), "JOINED" | "2")
    {
        return reply(tx, job, message!(message_catalog, "bot.target_02"), now);
    }
    if result
        .oc_result
        .as_ref()
        .and_then(|value| value.raw_member_name.as_deref())
        == Some(job.input.body.as_str())
    {
        return reply(
            tx,
            job,
            message!(
                message_catalog,
                "bot.target_03",
                arg0 = display_name(&job.input.body)
            ),
            now,
        );
    }
    let update = OcRequest::Profile {
        square_id: member.square_id.clone(),
        member_id: member.member_id.clone(),
        revision: member.revision.clone(),
        name: job.input.body.clone(),
    };
    job.operation = "bot-name".into();
    job.targets = vec![member.member_id.clone()];
    job.target_member = Some(member);
    request(tx, job, update, Phase::Mutation, now)
}

pub fn mutation(
    message_catalog: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let member = job.target_member.as_ref().ok_or("MissingBotProfile")?;
    let label = match result.status {
        DeliveryStatus::Sent => "成功",
        DeliveryStatus::Failed => "失敗",
        DeliveryStatus::Unknown => "結果不明",
    };
    history(
        message_catalog,
        tx,
        job,
        &member.member_id,
        label,
        &format!("{} → {} / {}", member.name, job.input.body, result.code),
        now,
    )?;
    reply(
        tx,
        job,
        match result.status {
            DeliveryStatus::Sent => message!(
                message_catalog,
                "bot.mutation_01",
                arg0 = display_name(&member.name),
                arg1 = display_name(&job.input.body)
            ),
            DeliveryStatus::Failed => {
                message!(message_catalog, "bot.mutation_02", arg0 = result.code)
            }
            DeliveryStatus::Unknown => {
                message!(message_catalog, "bot.mutation_03", arg0 = result.code)
            }
        },
        now,
    )
}

fn display_name(name: &str) -> String {
    // 結果表示だけを短縮・可視化する。変更APIへ渡す名前は加工しない。
    let preview = name.chars().take(80).collect::<String>();
    format!(
        "{}{}",
        preview.escape_debug(),
        if name.chars().nth(80).is_some() {
            "…"
        } else {
            ""
        }
    )
}
