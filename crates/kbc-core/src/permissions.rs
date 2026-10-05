use crate::Result;
use rusqlite::{Connection, params};
use serde::Deserialize;

const MAX_GRANTS: i64 = 4096;
const MAX_STOPS: i64 = 2048;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grant {
    chat_mid: String,
    user_mid: String,
    chat_type: String,
    role: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stop {
    kind: String,
    chat_mid: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    version: u32,
    roles: Vec<Grant>,
    #[serde(default)]
    bot_stops: Vec<Stop>,
    global_bot_stop: Option<serde_json::Value>,
}

pub fn initialize(db: &mut Connection, path: Option<&str>) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS bot_roles(scope TEXT NOT NULL,member TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('admin','mod')),actor TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(scope,member));
        CREATE TABLE IF NOT EXISTS bot_stops(chat TEXT PRIMARY KEY,actor TEXT NOT NULL,at INTEGER NOT NULL);")?;
    let imported: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='bot_permissions_imported')",
        [],
        |row| row.get(0),
    )?;
    if imported {
        return Ok(());
    }
    let Some(path) = path else {
        return Ok(());
    };
    if std::fs::metadata(path)?.len() > 256 * 1024 {
        return Err("PermissionFileLimit".into());
    }
    let text = std::fs::read_to_string(path)?;
    let file: File = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
    if file.version != 1
        || file.roles.len() > MAX_GRANTS as usize
        || file.bot_stops.len() > MAX_STOPS as usize
    {
        return Err("InvalidPermissionFile".into());
    }
    // 旧ファイルは初回だけ取り込む。解除済みの権限を再起動で戻さない。
    let tx = db.transaction()?;
    for grant in file.roles {
        if grant.chat_type != "SQUARE" {
            continue;
        }
        if grant.chat_mid.is_empty()
            || grant.chat_mid.len() > 256
            || !grant.user_mid.starts_with('p')
            || grant.user_mid.len() > 256
            || !matches!(grant.role.as_str(), "admin" | "mod")
        {
            return Err("InvalidPermissionGrant".into());
        }
        tx.execute("INSERT INTO bot_roles VALUES(?1,?2,?3,'legacy',0) ON CONFLICT(scope,member) DO UPDATE SET role='admin' WHERE excluded.role='admin'", params![grant.chat_mid,grant.user_mid,grant.role])?;
    }
    for stop in file.bot_stops {
        if stop.kind != "square" {
            continue;
        }
        if stop.chat_mid.is_empty() || stop.chat_mid.len() > 256 {
            return Err("InvalidBotStop".into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO bot_stops VALUES(?1,'legacy',0)",
            [stop.chat_mid],
        )?;
    }
    if file.global_bot_stop.is_some_and(|value| !value.is_null()) {
        tx.execute("INSERT OR IGNORE INTO bot_stops VALUES('*','legacy',0)", [])?;
    }
    tx.execute(
        "INSERT INTO metadata VALUES('bot_permissions_imported','1')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn rank(db: &Connection, scope: &str, alias: &str, member: &str) -> Result<u8> {
    Ok(db.query_row("SELECT COALESCE(max(CASE role WHEN 'admin' THEN 2 ELSE 1 END),0) FROM bot_roles WHERE scope IN (?1,?2) AND member=?3",params![scope,alias,member],|row|row.get(0))?)
}

pub fn set_role(
    db: &Connection,
    scope: &str,
    alias: &str,
    member: &str,
    role: &str,
    remove: bool,
    audit: (&str, i64),
) -> Result<bool> {
    let (actor, now) = audit;
    if remove {
        return Ok(db.execute(
            "DELETE FROM bot_roles WHERE scope IN (?1,?2) AND member=?3 AND role=?4",
            params![scope, alias, member, role],
        )? > 0);
    }
    let count: i64 = db.query_row(
        "SELECT count(*) FROM bot_roles WHERE scope IN (?1,?2) AND member=?3",
        params![scope, alias, member],
        |row| row.get(0),
    )?;
    let canonical: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM bot_roles WHERE scope=?1 AND member=?2 AND role=?3)",
        params![scope, member, role],
        |row| row.get(0),
    )?;
    if count == 1 && canonical {
        return Ok(false);
    }
    if count == 0
        && db.query_row("SELECT count(*) FROM bot_roles", [], |row| {
            row.get::<_, i64>(0)
        })? >= MAX_GRANTS
    {
        return Err("BotRoleCapacity".into());
    }
    db.execute(
        "DELETE FROM bot_roles WHERE scope IN (?1,?2) AND member=?3",
        params![scope, alias, member],
    )?;
    db.execute(
        "INSERT INTO bot_roles VALUES(?1,?2,?3,?4,?5)",
        params![scope, member, role, actor, now],
    )?;
    Ok(true)
}

pub fn stop_state(db: &Connection, chat: &str) -> Result<(bool, bool)> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM bot_stops WHERE chat='*'),EXISTS(SELECT 1 FROM bot_stops WHERE chat=?1)",[chat],|row|Ok((row.get(0)?,row.get(1)?)))?)
}
pub fn stopped(db: &Connection, chat: &str) -> Result<bool> {
    let (all, local) = stop_state(db, chat)?;
    Ok(all || local)
}
pub fn control(db: &Connection, chat: &str, stop: bool, actor: &str, now: i64) -> Result<bool> {
    if !stop {
        return Ok(db.execute("DELETE FROM bot_stops WHERE chat=?1", [chat])? > 0);
    }
    let existing: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM bot_stops WHERE chat=?1)",
        [chat],
        |row| row.get(0),
    )?;
    if existing {
        return Ok(false);
    }
    if db.query_row("SELECT count(*) FROM bot_stops", [], |row| {
        row.get::<_, i64>(0)
    })? >= MAX_STOPS
    {
        return Err("BotStopCapacity".into());
    }
    Ok(db.execute(
        "INSERT INTO bot_stops VALUES(?1,?2,?3)",
        params![chat, actor, now],
    )? > 0)
}
