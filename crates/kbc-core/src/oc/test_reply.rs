use super::*;

fn take_word<'a>(input: &mut &'a str) -> &'a str {
    *input = input.trim_start();
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    let word = &input[..end];
    *input = input[end..].trim_start();
    word
}

pub fn parse(text: &str) -> Option<Input> {
    let mut input = text;
    let command = take_word(&mut input);
    if (!command.eq_ignore_ascii_case("!test") && !command.eq_ignore_ascii_case("o.test"))
        || !take_word(&mut input).eq_ignore_ascii_case("reply")
        || input
            .split_whitespace()
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("help"))
    {
        return None;
    }
    // 本文の改行・連続空白を保つ。引数の解釈は権限確認後に行う。
    Some(Input {
        name: "test-reply".into(),
        args: vec![],
        body: input.into(),
    })
}

pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &Job, now: i64) -> Result<()> {
    if bot_rank(runtime, job) < 2 {
        return reply(tx, job, "このリプライ送信テストはBOT管理者専用です。", now);
    }
    let mut input = job.input.body.as_str();
    let message = take_word(&mut input);
    if message.is_empty()
        || message.len() > 32
        || !message.bytes().all(|byte| byte.is_ascii_digit())
    {
        return reply(
            tx,
            job,
            "返信先のメッセージIDを数字で指定してください。使い方: !test help",
            now,
        );
    }
    let mut chat = identity(&job.event).1;
    if input.split_whitespace().next() == Some("--to") {
        take_word(&mut input);
        chat = take_word(&mut input);
        if !chat.starts_with('m')
            || !(9..=64).contains(&chat.len())
            || !chat[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return reply(
                tx,
                job,
                "送信先はmから始まるトークMIDを指定してください。",
                now,
            );
        }
    }
    if input.split_whitespace().next() == Some("--") {
        take_word(&mut input);
    }
    if input.trim().is_empty() || input.encode_utf16().count() > 1500 {
        return reply(
            tx,
            job,
            "返信本文は1〜1,500 UTF-16単位で指定してください。",
            now,
        );
    }
    // 別OCの可否も調べるテストなので、未観測のIDを拒否せず実APIの結果を既存Outboxへ残す。
    text_action(
        tx,
        &job.event,
        "test-reply",
        chat,
        input.into(),
        TextDelivery {
            related_message_id: Some(message.into()),
            ..Default::default()
        },
        now,
    )?;
    Ok(())
}
