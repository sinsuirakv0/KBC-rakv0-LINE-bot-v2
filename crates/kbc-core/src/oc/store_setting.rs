use super::*;

pub fn parse(text: &str) -> Option<Input> {
    let mut words = text.split_whitespace();
    let command = words.next()?;
    if !command.eq_ignore_ascii_case("!pushsetting")
        && !command.eq_ignore_ascii_case("o.pushsetting")
    {
        return None;
    }
    let args: Vec<String> = words.take(4).map(str::to_ascii_lowercase).collect();
    if args.is_empty() {
        return None;
    }
    Some(Input {
        name: "store-setting".into(),
        args,
        body: String::new(),
    })
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &Job, now: i64) -> Result<()> {
    if !allowed(runtime, job, 1, 3) {
        return reply(
            tx,
            job,
            message!(&runtime.content.messages, "update.permission"),
            now,
        );
    }
    let body = crate::store_update::configure(runtime, tx, &job.event, &job.input.args)?;
    reply(tx, job, body, now)
}
