use super::*;
use serde_json::Value;

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("InvalidLegacyOc:{key}").into())
}
fn text(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_owned)
        .ok_or_else(|| format!("InvalidLegacyOc:{key}").into())
}
fn optional(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_owned)
}
fn flag(value: &Value, key: &str) -> Result<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("InvalidLegacyOc:{key}").into())
}
fn time(value: &Value, key: &str) -> Result<i64> {
    chrono::DateTime::parse_from_rfc3339(
        value
            .get(key)
            .and_then(Value::as_str)
            .ok_or("MissingLegacyTime")?,
    )
    .map(|v| v.timestamp_millis())
    .map_err(Into::into)
}

// 初回だけ、明示した旧管理設定を取り込む。新版で保存した設定は上書きしない。
pub fn import_legacy(db: &mut Connection, path: Option<&str>) -> Result<()> {
    let Some(path) = path else { return Ok(()) };
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='ocLegacyImported')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    if std::fs::metadata(path)?.len() > 2 * 1024 * 1024 {
        return Err("LegacyOcFileLimit".into());
    }
    let content = std::fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(content.trim_start_matches('\u{feff}'))?;
    if root.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("UnsupportedLegacyOcVersion".into());
    }
    let tx = db.transaction()?;
    for entry in array(&root, "settings")? {
        let square = text(entry, "squareMid")?;
        if !square.starts_with('s') {
            return Err("InvalidLegacySquare".into());
        }
        let existing: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM oc_settings WHERE square=?1)",
            [&square],
            |r| r.get(0),
        )?;
        if existing {
            continue;
        }
        let mut value = Settings {
            url: flag(entry, "linkDeleteEnabled")?,
            media: flag(entry, "mediaBurstDeleteEnabled")?,
            left: flag(entry, "leftSoonMonitoringEnabled")?,
            danger: flag(entry, "dangerWordAutoKickEnabled")?,
            cohort: flag(entry, "joinCohortWatchEnabled")?,
            report: flag(entry, "dangerWordAutoReportEnabled")?,
            main: optional(entry, "mainChatMid"),
            mod_room: optional(entry, "modRoomChatMid"),
            source_chat: optional(entry, "leftSoonSourceChatMid"),
            ..Default::default()
        };
        value.source_chat = value
            .source_chat
            .or_else(|| value.main.clone())
            .or_else(|| value.mod_room.clone());
        for rule in array(entry, "urlAllowRules")? {
            let raw = rule
                .get("sourceUrl")
                .and_then(Value::as_str)
                .ok_or("MissingLegacyUrl")?;
            let scope = rule
                .get("scope")
                .and_then(Value::as_str)
                .ok_or("MissingLegacyUrlScope")?;
            let rule = policy::UrlRule::new(raw, scope).ok_or("UnsupportedLegacyUrlRule")?;
            if !value.rules.contains(&rule) {
                value.rules.push(rule);
            }
        }
        for mute in array(entry, "mutes")? {
            let until = if mute.get("expiresAt").and_then(Value::as_str).is_some() {
                Some(time(mute, "expiresAt")?)
            } else {
                None
            };
            if until.is_some_and(|at| at <= crate::now_ms()) {
                continue;
            }
            let id = text(mute, "memberMid")?;
            value.mutes.insert(
                id,
                Mute {
                    until,
                    since: time(mute, "mutedAt")?,
                    name: mute
                        .get("displayName")
                        .and_then(Value::as_str)
                        .unwrap_or("メンバー")
                        .chars()
                        .take(80)
                        .collect(),
                },
            );
        }
        save_settings(&tx, &square, &value)?;
    }
    for (key, join) in [("joinMessages", true), ("leaveMessages", false)] {
        for entry in array(&root, key)? {
            let square = text(entry, "squareMid")?;
            let chat = text(entry, "squareChatMid")?;
            if !square.starts_with('s') || !chat.starts_with('m') {
                return Err("InvalidLegacyNotificationScope".into());
            }
            let existing: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM oc_notifications WHERE chat=?1)",
                [&chat],
                |r| r.get(0),
            )?;
            let mut value = notifications(&tx, &chat)?;
            if (join && value.join.is_some()) || (!join && value.leave.is_some()) {
                continue;
            }
            let body = entry
                .get("text")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 4096 && s.encode_utf16().count() <= 1300)
                .ok_or("LegacyNotificationLimit")?;
            let template = Some(Template {
                text: body.into(),
                mention: flag(entry, "mention")?,
                show_id: flag(entry, "showId")?,
            });
            if join {
                value.join = template
            } else {
                value.leave = template
            };
            tx.execute("INSERT INTO oc_notifications VALUES(?1,?2,?3) ON CONFLICT(chat) DO UPDATE SET payload=excluded.payload",params![chat,square,serde_json::to_string(&value)?])?;
            if !existing
                && tx.query_row("SELECT count(*) FROM oc_notifications", [], |r| {
                    r.get::<_, i64>(0)
                })? > 2048
            {
                return Err("LegacyNotificationCapacity".into());
            }
        }
    }
    tx.execute(
        "INSERT INTO metadata VALUES('ocLegacyImported',?1)",
        [crate::now_ms().to_string()],
    )?;
    tx.commit()?;
    Ok(())
}
