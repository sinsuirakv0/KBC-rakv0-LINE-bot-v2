//! 名前検索の複数候補を、共通Sessionの送信確定・ページ操作へ接続する。
use super::*;
use crate::commands::{
    pagination::{self, Input, PAGE_SIZE},
    sessions::Response,
};

#[derive(Serialize, Deserialize)]
struct Selection {
    members: Vec<OcMember>,
    page: usize,
    footer: String,
}

fn page(value: &Selection, messages: &crate::messages::Messages) -> String {
    let pages = value.members.len().div_ceil(PAGE_SIZE);
    let lines = value
        .members
        .iter()
        .skip(value.page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .enumerate()
        .map(|(index, member)| {
            let mut units = 0;
            let name = member
                .name
                .chars()
                .take_while(|c| {
                    units += c.len_utf16();
                    units <= 32
                })
                .collect::<String>();
            message!(
                messages,
                "id.choice",
                number = index + 1,
                name = name,
                member = member.member_id,
                state = member.state
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    message!(
        messages,
        "id.choices",
        page = value.page + 1,
        pages = pages,
        lines = lines,
        navigation = pagination::navigation(messages, value.page, pages),
        footer = value.footer
    )
}

pub(super) fn start(
    messages: &crate::messages::Messages,
    tx: &Transaction<'_>,
    job: &Job,
    members: Vec<OcMember>,
    footer: String,
    now: i64,
) -> Result<()> {
    let (id, chat, _, owner, _) = identity(&job.event);
    let value = Selection {
        members,
        page: 0,
        footer,
    };
    let payload = serde_json::to_string(&value)?;
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE chat=?1 AND owner=?2)",
        params![chat, owner],
        |row| row.get(0),
    )?;
    if payload.len() > 64 * 1024
        || (!existing
            && tx.query_row("SELECT count(*) FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })? >= 128)
    {
        return reply(tx, job, message!(messages, "search.apply_inner_03"), now);
    }
    let old: Option<String> = tx
        .query_row(
            "SELECT prompt FROM sessions WHERE chat=?1 AND owner=?2",
            params![chat, owner],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    tx.execute(
        "DELETE FROM sessions WHERE chat=?1 AND owner=?2",
        params![chat, owner],
    )?;
    let action = text_action(
        tx,
        &job.event,
        &format!("prompt-{}", job.step),
        chat,
        page(&value, messages),
        TextDelivery {
            replace: old,
            prompt: true,
            ..Default::default()
        },
        now,
    )?;
    tx.execute("INSERT INTO sessions(id,chat,owner,action,payload,expires,revision) VALUES(?1,?2,?3,?4,?5,?6,'id-v1')",
        params![id, chat, owner, action, payload, now + 600_000])?;
    Ok(())
}

pub(crate) fn select(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    id: &str,
    payload: &str,
    pending: bool,
    messages: &crate::messages::Messages,
    now: i64,
) -> Result<Vec<Response>> {
    let CoreEvent::MessageReceived { event_id, text, .. } = event else {
        return Ok(vec![]);
    };
    let mut value: Selection = serde_json::from_str(payload)?;
    let input = pagination::parse(text, value.page);
    let reply = |text: String| vec![(text, now, None)];
    if matches!(input, Input::Finish) {
        tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
        return Ok(reply(message!(messages, "search.apply_inner_04").into()));
    }
    if pending && matches!(input, Input::Select(_) | Input::Move(_)) {
        return Ok(reply(message!(messages, "search.apply_inner_05").into()));
    }
    match input {
        Input::Move(target) => {
            let pages = value.members.len().div_ceil(PAGE_SIZE);
            let Some(target) = target.filter(|target| *target < pages) else {
                return Ok(reply(message!(
                    messages,
                    "navigation.invalid_page",
                    pages = pages
                )));
            };
            if target == value.page {
                return Ok(vec![]);
            }
            value.page = target;
            // 新しい一覧が送信されるまでは、旧ページの番号との対応を維持する。
            tx.execute(
                "UPDATE sessions SET action=?2,pending_payload=?3 WHERE id=?1",
                params![id, format!("{event_id}:0"), serde_json::to_string(&value)?],
            )?;
            Ok(reply(page(&value, messages)))
        }
        Input::Select(number @ 1..=PAGE_SIZE) => {
            if let Some(member) = value.members.get(value.page * PAGE_SIZE + number - 1) {
                tx.execute("DELETE FROM sessions WHERE id=?1", [id])?;
                Ok(reply(person(messages, member)))
            } else {
                Ok(reply(message!(messages, "search.apply_inner_07").into()))
            }
        }
        Input::Select(_) => Ok(reply(message!(messages, "search.apply_inner_07").into())),
        _ => Ok(vec![]),
    }
}
