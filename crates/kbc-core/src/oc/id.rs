use super::*;
use unicode_normalization::UnicodeNormalization;

#[derive(Serialize, Deserialize)]
pub(super) struct Lookup {
    query: String,
    old: bool,
    debug: bool,
    state_index: usize,
    pages: usize,
    continuation_token: Option<String>,
    members: Vec<OcMember>,
}
#[derive(Serialize, Deserialize)]
struct MessageRef {
    message_id: String,
    chat_id: String,
    square_id: Option<String>,
    sender_id: Option<String>,
    sender_name: Option<String>,
    at: i64,
    related_message_id: Option<String>,
    related_service: Option<String>,
    relation: Option<String>,
}
pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS message_refs(chat TEXT,message TEXT,square TEXT,payload TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(chat,message));
        CREATE INDEX IF NOT EXISTS message_refs_at ON message_refs(at);
        CREATE INDEX IF NOT EXISTS message_refs_lookup ON message_refs(square,message);")?;
    Ok(())
}
pub fn remember(tx: &Transaction<'_>, event: &CoreEvent) -> Result<()> {
    let CoreEvent::MessageReceived {
        message_id,
        chat_id,
        square_id,
        sender_id,
        sender_name,
        reply_to_message_id,
        metadata_json,
        created_at_ms,
        ..
    } = event
    else {
        return Ok(());
    };
    let metadata: serde_json::Value = metadata_json
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    let value = MessageRef {
        message_id: message_id.clone(),
        chat_id: chat_id.clone(),
        square_id: square_id.clone(),
        sender_id: sender_id.clone(),
        sender_name: sender_name
            .as_ref()
            .map(|name| name.chars().take(80).collect()),
        at: *created_at_ms,
        related_message_id: reply_to_message_id.clone(),
        related_service: metadata.get("relatedMessageServiceCode").map(|v| {
            v.as_str()
                .map_or_else(|| v.to_string(), str::to_owned)
                .chars()
                .take(32)
                .collect()
        }),
        relation: metadata.get("messageRelationType").map(|v| {
            v.as_str()
                .map_or_else(|| v.to_string(), str::to_owned)
                .chars()
                .take(32)
                .collect()
        }),
    };
    if tx.query_row("SELECT count(*) FROM message_refs", [], |row| {
        row.get::<_, i64>(0)
    })? >= 8192
    {
        tx.execute("DELETE FROM message_refs WHERE rowid=(SELECT rowid FROM message_refs ORDER BY at LIMIT 1)", [])?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO message_refs VALUES(?1,?2,?3,?4,?5)",
        params![
            chat_id,
            message_id,
            square_id,
            serde_json::to_string(&value)?,
            created_at_ms
        ],
    )?;
    Ok(())
}
pub fn parse(text: &str) -> Option<Input> {
    let mut parts = text.split_whitespace();
    let name = parts.next()?;
    if !name.eq_ignore_ascii_case("!id") && !name.eq_ignore_ascii_case("o.id") {
        return None;
    }
    let args = parts.take(32).map(str::to_owned).collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|arg| arg.eq_ignore_ascii_case("help"))
    {
        return None;
    }
    Some(Input {
        name: "id".into(),
        body: args.join(" "),
        args,
    })
}
fn person(member: &OcMember) -> String {
    format!(
        "ユーザーID\n名前: {}\nMID: {}\n状態: {}",
        member.name, member.member_id, member.state
    )
}
fn normalized(text: &str) -> String {
    text.nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .collect()
}
fn matches_name(name: &str, query: &str) -> bool {
    let name = normalized(name);
    let query = normalized(query);
    if name.contains(&query) {
        return true;
    }
    let mut characters = name.chars();
    query
        .chars()
        .all(|wanted| characters.by_ref().any(|character| character == wanted))
}
pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let context = job.context.as_ref().ok_or("MissingIdContext")?.clone();
    let args = &job.input.args;
    let mode = args.first().map(String::as_str).unwrap_or("");
    if matches!(mode, "message" | "msg" | "reply" | "metadata") {
        return message_info(tx, job, mode == "reply", now);
    }
    if mode == "oc" {
        return reply(tx, job, format!("OC ID\nMID: {}", context.square_id), now);
    }
    if mode == "talk" {
        if args.get(1).is_some_and(|arg| arg == "oc") {
            if bot_rank(runtime, job) < 2 {
                return reply(tx, job, "参加中OC一覧はBOT管理者専用です。", now);
            }
            let token = args
                .iter()
                .position(|arg| arg == "--cursor")
                .and_then(|index| args.get(index + 1))
                .cloned();
            if token.as_ref().is_some_and(|token| token.len() > 2048) {
                return reply(tx, job, "継続tokenが長すぎます。", now);
            }
            job.operation = "id-chats".into();
            return request(
                tx,
                job,
                OcRequest::JoinedChats {
                    continuation_token: token,
                },
                Phase::Id,
                now,
            );
        }
        return reply(
            tx,
            job,
            format!(
                "トークID\nトーク名: {}\nMID: {}\n種別: SQUARE\n本OC MID: {}",
                context.chat_name,
                identity(&job.event).1,
                context.square_id
            ),
            now,
        );
    }
    if mode == "log" {
        return reply(
            tx,
            job,
            "入退室ログは自動保存されています。過去履歴の一括取得は今回のID移植とは別に扱います。",
            now,
        );
    }
    let old = mode == "old";
    if let CoreEvent::MessageReceived { mentions, .. } = &job.event {
        job.targets = mentions.clone().unwrap_or_default();
    }
    for arg in args {
        let id = arg.split_once(':').map_or(arg.as_str(), |(_, value)| value);
        if id.starts_with('p')
            && id.len() == 33
            && id[1..].bytes().all(|c| c.is_ascii_hexdigit())
            && !job.targets.iter().any(|target| target == id)
        {
            job.targets.push(id.into());
        }
    }
    if job.targets.len() > 8 {
        return reply(tx, job, "ID取得は一度に8人までです。", now);
    }
    if !job.targets.is_empty() {
        job.operation = "id-member".into();
        return next_member(tx, job, now);
    }
    if args.is_empty() || matches!(mode, "me" | "self" | "user" | "member") && args.len() == 1 {
        return reply(tx, job, person(&context.actor), now);
    }
    let debug = args.last().is_some_and(|arg| arg == "log");
    let query = args
        .iter()
        .skip(usize::from(old))
        .take(args.len() - usize::from(old) - usize::from(debug))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() || query.len() > 160 {
        return reply(
            tx,
            job,
            "名前を指定してください。!id help で使い方を確認できます。",
            now,
        );
    }
    // 保存済みの同じOCの名前も照合する。本文や全履歴を検索用に再取得しない。
    let mut statement = tx.prepare(
        "SELECT member,name,state FROM log_members WHERE square=?1 ORDER BY at DESC LIMIT 8192",
    )?;
    let cached = statement
        .query_map([&context.square_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut members = Vec::new();
    for (member_id, name, state) in cached {
        if matches_name(&name, &query) && (old || state == "JOINED") {
            members.push(OcMember {
                member_id,
                name,
                square_id: context.square_id.clone(),
                role: String::new(),
                state,
                revision: "0".into(),
            });
            if members.len() == 20 {
                break;
            }
        }
    }
    job.id_lookup = Some(Lookup {
        query,
        old,
        debug,
        state_index: 0,
        pages: 0,
        continuation_token: None,
        members,
    });
    search_page(tx, job, now)
}
fn next_member(tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    if let Some(member_id) = job.targets.first().cloned() {
        request(tx, job, OcRequest::Member { member_id }, Phase::Id, now)
    } else {
        reply(tx, job, job.results.join("\n\n"), now)
    }
}
fn search_page(tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let lookup = job.id_lookup.as_ref().ok_or("MissingIdLookup")?;
    let states = if lookup.old {
        &['L', 'K', 'B', 'J'][..]
    } else {
        &['J'][..]
    };
    let state = match states[lookup.state_index] {
        'L' => "LEFT",
        'K' => "KICK_OUT",
        'B' => "BANNED",
        _ => "JOINED",
    };
    let api = OcRequest::Members {
        square_id: job
            .context
            .as_ref()
            .ok_or("MissingIdContext")?
            .square_id
            .clone(),
        query: lookup.query.nfkc().collect(),
        state: state.into(),
        continuation_token: lookup.continuation_token.clone(),
    };
    request(tx, job, api, Phase::Id, now)
}
pub fn complete(
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    let response = result.oc_result.as_ref().ok_or("MissingIdResult")?;
    let square = &job.context.as_ref().ok_or("MissingIdContext")?.square_id;
    if job.operation == "id-chats" {
        let mut lines = vec!["参加中OpenChat一覧".into()];
        for chat in &response.chats {
            lines.push(format!(
                "{} / {}\nトークMID: {}\nOC MID: {}",
                if chat.is_main { "本OC" } else { "サブOC" },
                chat.name,
                chat.chat_id,
                chat.square_id
            ));
        }
        if let Some(token) = &response.continuation_token {
            lines.push(format!("続き: !id talk oc --cursor {token}"));
        }
        return reply(tx, job, lines.join("\n\n"), now);
    }
    if job.operation == "id-member" {
        let member = response.member.as_ref().ok_or("MissingIdMember")?;
        if &member.square_id != square || job.targets.first() != Some(&member.member_id) {
            return reply(tx, job, "同じOCのメンバーを指定してください。", now);
        }
        job.results.push(person(member));
        job.targets.remove(0);
        return next_member(tx, job, now);
    }
    let lookup = job.id_lookup.as_mut().ok_or("MissingIdLookup")?;
    for member in &response.members {
        if &member.square_id != square {
            return Err("IdMemberScopeMismatch".into());
        }
        if matches_name(&member.name, &lookup.query) {
            if let Some(previous) = lookup
                .members
                .iter_mut()
                .find(|value| value.member_id == member.member_id)
            {
                *previous = member.clone();
            } else if lookup.members.len() < 20 {
                lookup.members.push(member.clone());
            }
        }
    }
    lookup.pages += 1;
    let continued = response
        .continuation_token
        .as_ref()
        .is_some_and(|token| Some(token) != lookup.continuation_token.as_ref());
    if continued && lookup.pages < 4 && lookup.members.len() < 20 {
        lookup.continuation_token = response.continuation_token.clone();
        return search_page(tx, job, now);
    }
    if lookup.old && lookup.state_index < 3 && lookup.members.len() < 20 {
        lookup.state_index += 1;
        lookup.pages = 0;
        lookup.continuation_token = None;
        return search_page(tx, job, now);
    }
    let mut text = if lookup.members.is_empty() {
        "該当するメンバーが見つかりませんでした。".into()
    } else {
        lookup
            .members
            .iter()
            .map(person)
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    if continued || lookup.members.len() >= 20 {
        text.push_str("\n\n取得上限に達しました。名前を詳しく指定して絞り込んでください。");
    }
    if lookup.debug {
        text.push_str(&format!(
            "\n\n検索情報\n状態数: {}\n最終状態のページ数: {}\n表示件数: {}",
            lookup.state_index + 1,
            lookup.pages,
            lookup.members.len()
        ));
    }
    reply(tx, job, text, now)
}
fn message_info(tx: &Transaction<'_>, job: &Job, use_reply: bool, now: i64) -> Result<()> {
    let CoreEvent::MessageReceived {
        reply_to_message_id,
        ..
    } = &job.event
    else {
        return Ok(());
    };
    let explicit = job.input.args.get(1).filter(|arg| !arg.starts_with("--"));
    let target = explicit.map(String::as_str).or(if use_reply {
        reply_to_message_id.as_deref()
    } else {
        Some(identity(&job.event).2)
    });
    let Some(target) = target else {
        return reply(
            tx,
            job,
            "対象メッセージへ !id reply とリプライするか、!id message <message ID> を指定してください。",
            now,
        );
    };
    if target.len() > 256 {
        return reply(tx, job, "message IDが長すぎます。", now);
    }
    let chat = job
        .input
        .args
        .iter()
        .position(|arg| arg == "--chat")
        .and_then(|index| job.input.args.get(index + 1));
    let square = &job.context.as_ref().ok_or("MissingIdContext")?.square_id;
    let payload: Option<String> = tx.query_row("SELECT payload FROM message_refs WHERE square=?1 AND message=?2 AND (?3 IS NULL OR chat=?3) AND at>?4 ORDER BY at DESC LIMIT 1",
        params![square,target,chat,now - 48 * 3600000], |row| row.get(0)).optional()?;
    let mut lines = vec![
        format!("メッセージ情報\nmessage ID: {target}"),
        format!(
            "リプライ指定\nrelatedMessageId: {target}\nrelatedMessageServiceCode: SQUARE\nmessageRelationType: REPLY"
        ),
    ];
    if let Some(payload) = payload {
        let value: MessageRef = serde_json::from_str(&payload)?;
        lines.push(format!(
            "元トークMID: {}\nOC MID: {}\n送信者MID: {}\n送信者名: {}\n送信時刻(ms): {}",
            value.chat_id,
            value.square_id.as_deref().unwrap_or("未取得"),
            value.sender_id.as_deref().unwrap_or("未取得"),
            value.sender_name.as_deref().unwrap_or("未取得"),
            value.at
        ));
        if let Some(related) = value.related_message_id {
            lines.push(format!(
                "元メッセージの返信先ID: {related}\nservice: {}\nrelation: {}",
                value.related_service.as_deref().unwrap_or("未取得"),
                value.relation.as_deref().unwrap_or("未取得")
            ));
        }
    } else {
        lines.push(
            "元トーク・送信者は未観測です。直近48時間・8,192件までの受信済み情報を参照します。"
                .into(),
        );
    }
    reply(tx, job, lines.join("\n\n"), now)
}
