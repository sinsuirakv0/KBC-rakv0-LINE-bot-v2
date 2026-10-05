use super::*;
use crate::messages::message;

pub(super) fn take_word<'a>(input: &mut &'a str) -> &'a str {
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
    let message_catalog = &runtime.content.messages;
    if bot_rank(tx, job)? < 2 {
        return reply(
            tx,
            job,
            message!(message_catalog, "common.bot_admin_only"),
            now,
        );
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
            message!(message_catalog, "test_reply.execute_01"),
            now,
        );
    }
    let chat = identity(&job.event).1;
    if input.split_whitespace().next() == Some("--to") {
        return reply(
            tx,
            job,
            message!(message_catalog, "test_reply.execute_02"),
            now,
        );
    }
    if input.split_whitespace().next() == Some("--chat") {
        take_word(&mut input);
        let source_chat = take_word(&mut input);
        if !source_chat.starts_with('m')
            || !(9..=64).contains(&source_chat.len())
            || !source_chat[1..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return reply(
                tx,
                job,
                message!(message_catalog, "test_reply.execute_03"),
                now,
            );
        }
        // SDKに返信元MIDの引数はない。観測済みのIDとの矛盾だけを検査し、履歴取得は増やさない。
        let wrong_chat: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_refs WHERE message=?1 AND chat<>?2 AND at>?3)
                AND NOT EXISTS(SELECT 1 FROM message_refs WHERE chat=?2 AND message=?1 AND at>?3)",
            params![message, source_chat, now - 48 * 3600000],
            |row| row.get(0),
        )?;
        if wrong_chat {
            return reply(
                tx,
                job,
                message!(message_catalog, "test_reply.execute_04"),
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
            message!(message_catalog, "test_reply.execute_05"),
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
