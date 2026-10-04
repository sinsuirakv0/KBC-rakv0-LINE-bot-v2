use super::*;

pub fn parse(text: &str) -> Option<Input> {
    let mut input = text;
    let command = test_reply::take_word(&mut input);
    if (!command.eq_ignore_ascii_case("!bot") && !command.eq_ignore_ascii_case("o.bot"))
        || !test_reply::take_word(&mut input).eq_ignore_ascii_case("name")
    {
        return None;
    }
    // 名前の途中の空白は保ち、前後だけ除く。
    Some(Input {
        name: "bot-name".into(),
        args: vec![],
        body: input.trim().into(),
    })
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    if bot_rank(runtime, job) < 2 {
        return reply(tx, job, "Botの名前変更はBOT管理者専用です。", now);
    }
    let name = &job.input.body;
    if name.is_empty() || name.encode_utf16().count() > 20 || name.chars().any(char::is_control) {
        return reply(
            tx,
            job,
            "使い方: !bot name 名前\n名前は20文字以内（絵文字は2文字分の場合があります）。改行・制御文字は使えません。",
            now,
        );
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
    // 変更APIの発行時も、BOT管理者としての権限を確認する。
    if bot_rank(runtime, job) < 2 {
        return reply(tx, job, "Botの名前変更はBOT管理者専用です。", now);
    }
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    let Some(member) = result
        .oc_result
        .as_ref()
        .and_then(|value| value.member.clone())
    else {
        return reply(tx, job, "Botのプロフィールを確認できませんでした。", now);
    };
    if member.member_id != context.bot_member_id
        || member.square_id != context.square_id
        || !matches!(member.state.as_str(), "JOINED" | "2")
    {
        return reply(tx, job, "このOCに参加中のBotを確認できませんでした。", now);
    }
    if member.name == job.input.body {
        return reply(
            tx,
            job,
            format!("Botの名前はすでに「{}」です。", member.name),
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

pub fn mutation(tx: &Transaction<'_>, job: &Job, result: &ActionResult, now: i64) -> Result<()> {
    let member = job.target_member.as_ref().ok_or("MissingBotProfile")?;
    let label = match result.status {
        DeliveryStatus::Sent => "成功",
        DeliveryStatus::Failed => "失敗",
        DeliveryStatus::Unknown => "結果不明",
    };
    history(
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
            DeliveryStatus::Sent => format!(
                "Botの名前を変更しました。\n{} → {}",
                member.name, job.input.body
            ),
            DeliveryStatus::Failed => {
                format!("Botの名前を変更できませんでした。\nAPI: {}", result.code)
            }
            DeliveryStatus::Unknown => format!(
                "Botの名前変更: 結果不明\nAPI: {}\nプロフィールを確認してください。自動再実行はしません。",
                result.code
            ),
        },
        now,
    )
}
