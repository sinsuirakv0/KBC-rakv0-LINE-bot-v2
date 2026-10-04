use crate::Result;
use kbc_protocol::{CoreEvent, PendingLog};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS log_pending(sequence INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE,stream TEXT NOT NULL,row TEXT NOT NULL,bytes INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS log_members(square TEXT,member TEXT,name TEXT,at INTEGER,state TEXT,PRIMARY KEY(square,member));
        CREATE INDEX IF NOT EXISTS log_member_at ON log_members(at);")?;
    Ok(())
}
fn valid_mid(mid: &str) -> bool {
    !mid.is_empty() && mid.len() <= 128 && mid.chars().all(|c| c.is_ascii_alphanumeric())
}
fn append(tx: &Transaction<'_>, id: &str, stream: &str, mut row: Vec<Value>) -> Result<()> {
    while row.last().is_some_and(Value::is_null) {
        row.pop();
    }
    let row = serde_json::to_string(&row)?;
    if row.len() > 64 * 1024 {
        return Err("LogRecordLimit".into());
    }
    tx.execute(
        "INSERT OR IGNORE INTO log_pending(id,stream,row,bytes) VALUES(?1,?2,?3,?4)",
        params![id, stream, row, row.len() as i64],
    )?;
    Ok(())
}
pub fn ingest(tx: &Transaction<'_>, event: &CoreEvent) -> Result<()> {
    match event {
        CoreEvent::ReactionNotified { .. } => {}
        CoreEvent::MessageReceived {
            event_id,
            chat_id,
            message_id,
            sender_id,
            text,
            content_type,
            sender_name,
            metadata_json,
            square_id,
            created_at_ms,
            ..
        } => {
            if !valid_mid(chat_id) {
                return Err("InvalidLogScope".into());
            }
            let square = square_id
                .as_deref()
                .filter(|s| s.starts_with('s') && valid_mid(s));
            let base = square.map_or_else(
                || format!("unmapped/{chat_id}"),
                |s| format!("{s}/{chat_id}"),
            );
            let metadata: Value = metadata_json
                .as_ref()
                .map(|s| serde_json::from_str(s))
                .transpose()?
                .unwrap_or(Value::Null);
            append(
                tx,
                event_id,
                &format!("{base}/messages"),
                vec![
                    json!(created_at_ms),
                    json!(message_id),
                    json!(sender_id),
                    json!(text),
                    json!(content_type),
                    json!(sender_name),
                    if metadata.is_null() {
                        Value::Null
                    } else {
                        json!([metadata, 0])
                    },
                ],
            )?;
            if let (Some(square), Some(member), Some(name)) =
                (square, sender_id.as_deref(), sender_name.as_deref())
            {
                record_member(tx, event_id, square, member, name, "", *created_at_ms)?;
            }
        }
        CoreEvent::MemberChanged {
            event_id,
            square_id,
            chat_id,
            member_id,
            display_name,
            scope,
            state,
            metadata_json,
            created_at_ms,
            ..
        } => {
            if !valid_mid(square_id) || !valid_mid(chat_id) || !valid_mid(member_id) {
                return Err("InvalidLogScope".into());
            }
            let previous = record_member(
                tx,
                event_id,
                square_id,
                member_id,
                display_name,
                if scope == "square" && state != "NAME" {
                    state
                } else {
                    ""
                },
                *created_at_ms,
            )?;
            if state != "NAME"
                && matches!(state.as_str(), "JOINED" | "LEFT" | "KICK_OUT" | "BANNED")
                && previous.as_ref().is_none_or(|(_, at, old)| {
                    at <= created_at_ms && (scope != "square" || old != state)
                })
            {
                let base = if scope == "square" {
                    square_id.clone()
                } else {
                    format!("{square_id}/{chat_id}")
                };
                let kind = match state.as_str() {
                    "JOINED" => "join",
                    "LEFT" => "leave",
                    "KICK_OUT" => "kick",
                    _ => "ban",
                };
                append(
                    tx,
                    &format!("{event_id}:member"),
                    &format!("{base}/member-events"),
                    vec![
                        json!(created_at_ms),
                        json!(kind),
                        json!(member_id),
                        json!(display_name),
                        metadata_json
                            .as_deref()
                            .map(serde_json::from_str)
                            .transpose()?
                            .unwrap_or(Value::Null),
                        if scope == "square" && chat_id != square_id {
                            json!(chat_id)
                        } else {
                            Value::Null
                        },
                    ],
                )?;
            }
        }
    }
    Ok(())
}
type MemberState = (String, i64, String);
fn record_member(
    tx: &Transaction<'_>,
    id: &str,
    square: &str,
    member: &str,
    name: &str,
    state: &str,
    at: i64,
) -> Result<Option<MemberState>> {
    let previous: Option<MemberState> = tx
        .query_row(
            "SELECT name,at,state FROM log_members WHERE square=?1 AND member=?2",
            params![square, member],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if previous.as_ref().is_some_and(|(_, old, _)| *old > at) {
        return Ok(previous);
    }
    if !name.is_empty() && previous.as_ref().is_none_or(|(old, _, _)| old != name) {
        append(
            tx,
            &format!("{id}:name"),
            &format!("{square}/names"),
            vec![
                json!(at),
                json!(member),
                json!(previous.as_ref().map(|(name, _, _)| name)),
                json!(name),
            ],
        )?;
    }
    let name = if name.is_empty() {
        previous
            .as_ref()
            .map(|(name, _, _)| name.as_str())
            .unwrap_or("")
    } else {
        name
    };
    let state = if state.is_empty() {
        previous
            .as_ref()
            .map(|(_, _, state)| state.as_str())
            .unwrap_or("")
    } else {
        state
    };
    tx.execute("INSERT INTO log_members VALUES(?1,?2,?3,?4,?5) ON CONFLICT(square,member) DO UPDATE SET name=excluded.name,at=excluded.at,state=excluded.state",params![square,member,name,at,state])?;
    Ok(previous)
}
pub fn check_capacity(tx: &Transaction<'_>) -> Result<()> {
    tx.execute("DELETE FROM log_members WHERE rowid IN (SELECT rowid FROM log_members ORDER BY at DESC LIMIT -1 OFFSET 8192)",[])?;
    let (count, bytes): (i64, i64) = tx.query_row(
        "SELECT count(*),COALESCE(sum(bytes),0) FROM log_pending",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if count > 8192 || bytes > 8 * 1024 * 1024 {
        return Err("PendingLogCapacity".into());
    }
    Ok(())
}
pub fn pending(db: &Connection) -> Result<Vec<PendingLog>> {
    let mut statement =
        db.prepare("SELECT sequence,stream,row FROM log_pending ORDER BY sequence LIMIT 8192")?;
    let mut result = Vec::new();
    let mut bytes = 0;
    for row in statement.query_map([], |r| {
        Ok(PendingLog {
            sequence: r.get(0)?,
            stream: r.get(1)?,
            row: r.get(2)?,
        })
    })? {
        let row = row?;
        bytes += row.row.len();
        if bytes > 8 * 1024 * 1024 {
            break;
        }
        result.push(row);
    }
    Ok(result)
}
pub fn acknowledge(db: &mut Connection, sequences: Vec<u32>) -> Result<()> {
    if sequences.len() > 128 {
        return Err("LogAckLimit".into());
    }
    let tx = db.transaction()?;
    for sequence in sequences {
        tx.execute("DELETE FROM log_pending WHERE sequence=?1", [sequence])?;
    }
    tx.commit()?;
    Ok(())
}
