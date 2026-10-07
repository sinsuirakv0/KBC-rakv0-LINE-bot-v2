use crate::{Result, Runtime, messages::message, now_ms, oc::identity};
use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use kbc_protocol::CoreEvent;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use source::{
    StorePlatform, StoreVersionSource, asset_version_code, compare_versions, valid_version,
};
use std::{cmp::Ordering, sync::Arc, time::Duration};

mod source;
const MAX_SUBSCRIPTIONS: i64 = 512;

#[derive(Clone, Copy)]
enum Topic {
    Store(StorePlatform),
    Schedule,
}
impl Topic {
    fn key(self) -> &'static str {
        match self {
            Self::Store(platform) => platform.key(),
            Self::Schedule => "skd",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Store(platform) => platform.label(),
            Self::Schedule => "skd",
        }
    }
}

pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS store_subscriptions(platform TEXT NOT NULL,chat TEXT NOT NULL,PRIMARY KEY(platform,chat));
        CREATE TABLE IF NOT EXISTS store_versions(platform TEXT PRIMARY KEY,version TEXT NOT NULL,checked INTEGER NOT NULL,error TEXT NOT NULL DEFAULT '');")?;
    Ok(())
}

pub fn configure(
    runtime: &Runtime,
    tx: &Transaction<'_>,
    event: &CoreEvent,
    args: &[String],
) -> Result<String> {
    let catalog = &runtime.content.messages;
    let chat = identity(event).1;
    if args == ["status"] {
        return [
            Topic::Store(StorePlatform::Android),
            Topic::Store(StorePlatform::Ios),
            Topic::Schedule,
        ]
        .into_iter()
        .map(|platform| status(runtime, tx, platform, chat))
        .collect::<Result<Vec<_>>>()
        .map(|rows| rows.join("\n\n"));
    }
    let platforms = args
        .first()
        .and_then(|value| value.split(',').map(platform).collect::<Option<Vec<_>>>());
    let Some(mut platforms) = platforms.filter(|values| !values.is_empty() && values.len() <= 3)
    else {
        return Ok(runtime
            .content
            .command_help("pushsetting")
            .unwrap_or_default());
    };
    platforms.sort_by_key(|platform| platform.key());
    platforms.dedup_by_key(|platform| platform.key());
    let operation = args.get(1).map_or("on", String::as_str);
    if args.len() > 2 || !matches!(operation, "on" | "off" | "status") {
        return Ok(runtime
            .content
            .command_help("pushsetting")
            .unwrap_or_default());
    }
    if operation == "on" {
        let count: i64 = tx.query_row("SELECT count(*) FROM store_subscriptions", [], |row| {
            row.get(0)
        })?;
        let mut additional = 0;
        for platform in &platforms {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform=?1 AND chat=?2)",
                params![platform.key(), chat],
                |row| row.get(0),
            )?;
            if !exists {
                additional += 1;
            }
        }
        if count + additional > MAX_SUBSCRIPTIONS {
            return Ok(message!(catalog, "update.capacity").into());
        }
    }
    let mut rows = Vec::new();
    for platform in platforms {
        if operation == "status" {
            rows.push(status(runtime, tx, platform, chat)?);
            continue;
        }
        if operation == "on" {
            tx.execute(
                "INSERT OR IGNORE INTO store_subscriptions VALUES(?1,?2)",
                params![platform.key(), chat],
            )?;
        } else {
            tx.execute(
                "DELETE FROM store_subscriptions WHERE platform=?1 AND chat=?2",
                params![platform.key(), chat],
            )?;
            if platform.key() == "skd" {
                tx.execute("DELETE FROM schedule_targets WHERE chat=?1", [chat])?;
                tx.execute("DELETE FROM schedule_updates WHERE NOT EXISTS(SELECT 1 FROM schedule_targets t WHERE t.timestamp=schedule_updates.timestamp)",[])?;
            }
            // 未送信の通知だけ取消す。通信開始後の成否は従来のOutboxへ任せる。
            tx.execute(
                "DELETE FROM actions WHERE status='queued' AND chat=?1 AND event_id LIKE ?2",
                params![chat, format!("store:{}:%", platform.key())],
            )?;
        }
        rows.push(message!(
            catalog,
            "update.configured",
            platform = platform.label(),
            state = if operation == "on" {
                message!(catalog, "update.on")
            } else {
                message!(catalog, "update.off")
            }
        ));
    }
    Ok(rows.join("\n"))
}

fn platform(value: &str) -> Option<Topic> {
    match value {
        "android" => Some(Topic::Store(StorePlatform::Android)),
        "ios" => Some(Topic::Store(StorePlatform::Ios)),
        "skd" => Some(Topic::Schedule),
        _ => None,
    }
}

fn status(runtime: &Runtime, db: &Connection, platform: Topic, chat: &str) -> Result<String> {
    let catalog = &runtime.content.messages;
    let enabled: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform=?1 AND chat=?2)",
        params![platform.key(), chat],
        |row| row.get(0),
    )?;
    let row: Option<(String, i64, String)> = db
        .query_row(
            "SELECT version,checked,error FROM store_versions WHERE platform=?1",
            [platform.key()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (mut version, checked, error) = row.unwrap_or((
        message!(catalog, "update.unobserved").into(),
        0,
        String::new(),
    ));
    if version.is_empty() {
        version = message!(catalog, "update.unobserved").into();
    }
    if platform.key() == "skd" {
        version = crate::skd::monitor::display_watermark(&version)
            .unwrap_or_else(|| message!(catalog, "update.unobserved").into());
    }
    let mut pending: i64 = db.query_row("SELECT count(*) FROM actions WHERE chat=?1 AND event_id LIKE ?2 AND status NOT IN ('sent','failed')",params![chat,format!("store:{}:%",platform.key())],|row|row.get(0))?;
    if platform.key() == "skd" {
        pending += db.query_row(
            "SELECT count(*) FROM schedule_targets WHERE chat=?1",
            [chat],
            |row| row.get::<_, i64>(0),
        )?;
    }
    Ok(message!(
        catalog,
        "update.status",
        platform = platform.label(),
        state = if enabled {
            message!(catalog, "update.on")
        } else {
            message!(catalog, "update.off")
        },
        version = version,
        checked = if checked > 0 {
            detected_at(checked)?
        } else {
            message!(catalog, "update.unobserved").into()
        },
        pending = pending,
        error = if error.is_empty() {
            message!(catalog, "update.no_error")
        } else {
            &error
        }
    ))
}

impl Runtime {
    pub async fn run_store_monitors(&self) -> Result<()> {
        if self
            .store_worker_active
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Err("StoreWorkerAlreadyRunning".into());
        }
        tokio::join!(
            self.store_loop(StorePlatform::Android),
            self.store_loop(StorePlatform::Ios),
            self.schedule_loop(),
            self.push_loop()
        );
        self.store_worker_active
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    async fn store_loop(&self, platform: StorePlatform) {
        let mut source = StoreVersionSource::new(platform, Arc::new(self.assets.clone()));
        let mut failures = 0_usize;
        loop {
            let result = tokio::select! {
                _ = self.cancel.cancelled() => return,
                result = self.poll_store(platform,&mut source) => result,
            };
            let delay = match result {
                Ok(()) => {
                    failures = 0;
                    5
                }
                Err(error) => {
                    eprintln!("Store monitor {}: {error}", platform.key());
                    if let Ok(db) = self.database.lock() {
                        let _ = db.execute("INSERT INTO store_versions(platform,version,checked,error) VALUES(?1,'',0,'StoreCheckFailed') ON CONFLICT(platform) DO UPDATE SET error=excluded.error WHERE error<>excluded.error",[platform.key()]);
                    }
                    let delay = [15, 60, 300][failures.min(2)];
                    failures = failures.saturating_add(1);
                    delay
                }
            };
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
            }
        }
    }

    async fn poll_store(
        &self,
        platform: StorePlatform,
        source: &mut StoreVersionSource,
    ) -> Result<()> {
        let previous: Option<String> = {
            let db = self.database.lock().map_err(|_| "DatabaseLock")?;
            let active: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform=?1)",
                [platform.key()],
                |row| row.get(0),
            )?;
            if !active {
                return Ok(());
            }
            db.query_row(
                "SELECT NULLIF(version,'') FROM store_versions WHERE platform=?1",
                [platform.key()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
        };
        let version = source.fetch().await?;
        let confirmed = if previous
            .as_ref()
            .is_some_and(|old| compare_versions(&version, old) == Some(Ordering::Greater))
        {
            source.fetch().await? == version
        } else {
            false
        };
        self.record_store(platform, &version, confirmed, now_ms())
    }

    fn record_store(
        &self,
        platform: StorePlatform,
        version: &str,
        confirmed: bool,
        now: i64,
    ) -> Result<()> {
        if !valid_version(version) {
            return Err("InvalidStoreVersion".into());
        }
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT NULLIF(version,'') FROM store_versions WHERE platform=?1",
                [platform.key()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let mut statement = tx.prepare(
            "SELECT chat FROM store_subscriptions WHERE platform=?1 ORDER BY chat LIMIT 513",
        )?;
        let mut chats = statement
            .query_map([platform.key()], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let mut active = Vec::new();
        for chat in chats {
            if !crate::permissions::stopped(&tx, &chat)? {
                active.push(chat);
            }
        }
        chats = active;

        if chats.len() > MAX_SUBSCRIPTIONS as usize {
            return Err("StoreSubscriptionCapacity".into());
        }
        if let Some(previous) = previous.as_deref() {
            match compare_versions(version, previous) {
                Some(Ordering::Less) | None => return Err("StoreVersionRegressed".into()),
                Some(Ordering::Greater) => {
                    if !confirmed {
                        return Err("StoreVersionUnconfirmed".into());
                    }
                    let pending = crate::skd::pending_count(&tx)?;
                    if pending + chats.len() as i64 > crate::MAX_ACTIONS {
                        return Err("StoreCapacity".into());
                    }
                    let body = notification(self, platform, previous, version, now)?;
                    let event = format!("store:{}:{version}", platform.key());
                    // 検知版と通知登録を同じtransactionで確定し、入力なしでも既存配送を起こす。
                    for chat in chats {
                        self.enqueue_responses(
                            &tx,
                            &format!("{event}:{chat}"),
                            &chat,
                            vec![(body.clone(), now, None)],
                            None,
                            now,
                        )?;
                    }
                }
                Some(Ordering::Equal) => {}
            }
        }
        // 5秒ごとのfsyncは避け、変化・復旧または1分ごとに観測時刻を保存する。
        tx.execute("INSERT INTO store_versions(platform,version,checked,error) VALUES(?1,?2,?3,'') ON CONFLICT(platform) DO UPDATE SET version=excluded.version,checked=excluded.checked,error='' WHERE version<>excluded.version OR error<>'' OR checked<=?3-60000",params![platform.key(),version,now])?;
        tx.commit()?;
        drop(db);
        self.wake.notify_waiters();
        Ok(())
    }
}

pub(crate) fn detected_at(now: i64) -> Result<String> {
    let date: DateTime<Utc> = DateTime::from_timestamp_millis(now).ok_or("InvalidStoreTime")?;
    let date = date.with_timezone(&FixedOffset::east_opt(9 * 3600).ok_or("InvalidTimezone")?);
    let weekday =
        ["日", "月", "火", "水", "木", "金", "土"][date.weekday().num_days_from_sunday() as usize];
    Ok(format!(
        "{:04}/{:02}/{:02}({weekday}) {:02}:{:02}:{:02}",
        date.year(),
        date.month(),
        date.day(),
        date.hour(),
        date.minute(),
        date.second()
    ))
}

fn notification(
    runtime: &Runtime,
    platform: StorePlatform,
    previous: &str,
    version: &str,
    now: i64,
) -> Result<String> {
    let catalog = &runtime.content.messages;
    let code = asset_version_code(version).ok_or("InvalidAssetVersion")?;
    let compare = asset_version_code(previous).ok_or("InvalidAssetVersion")?;
    let body = message!(
        catalog,
        "update.notification",
        platform = platform.key(),
        version = version,
        detected_at = detected_at(now)?,
        asset_url = format!(
            "https://kbc-rakv0.vercel.app/pages/asset-explorer/?dataset=Local&version={code}&compare={compare}&view=diff&layout=grid&offset=200"
        ),
        store = platform.store_name(),
        store_url = platform.store_url()
    );
    if body.encode_utf16().count() > 1500 {
        return Err("StoreNotificationTooLong".into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(path: &std::path::Path) -> Runtime {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Runtime::open(serde_json::from_value(serde_json::json!({
            "databasePath": path, "ownerId": "store-test", "contentDirectory": root.join("content")
        })).unwrap()).unwrap()
    }

    #[test]
    fn persists_baseline_and_atomic_notifications() {
        let directory =
            std::env::temp_dir().join(format!("kbc-store-{}-{}", std::process::id(), now_ms()));
        let path = directory.join("core.sqlite");
        let core = runtime(&path);
        {
            let db = core.database.lock().unwrap();
            db.execute("INSERT INTO store_subscriptions VALUES('android','m1'),('android','m2'),('ios','m1')",[]).unwrap();
        }
        let now = now_ms();
        core.record_store(StorePlatform::Android, "15.6.0", false, now)
            .unwrap();
        core.record_store(StorePlatform::Ios, "15.6.1", false, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 0);
        assert!(
            core.record_store(StorePlatform::Android, "15.7.0", false, now)
                .is_err()
        );
        core.record_store(StorePlatform::Android, "15.7.0", true, now)
            .unwrap();
        {
            let db = core.database.lock().unwrap();
            let payload: String = db
                .query_row("SELECT payload FROM actions LIMIT 1", [], |row| row.get(0))
                .unwrap();
            let action: kbc_protocol::CoreAction = serde_json::from_str(&payload).unwrap();
            let kbc_protocol::CoreAction::SendMessage {
                text,
                mention,
                related_message_id,
                ..
            } = action
            else {
                panic!("wrong action")
            };
            assert!(mention.is_none() && related_message_id.is_empty());
            assert!(
                text.contains("android Ver.15.7.0")
                    && text.contains("version=150700&compare=150600")
            );
        }
        drop(core);
        let core = runtime(&path);
        core.record_store(StorePlatform::Android, "15.7.0", true, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 2);
        assert!(
            core.record_store(StorePlatform::Android, "15.6.0", false, now)
                .is_err()
        );
        core.record_store(StorePlatform::Ios, "15.7.0", true, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 3);
        // 停止中の更新は登録しない。再開時に停止期間の更新を再通知しない。
        {
            let db = core.database.lock().unwrap();
            crate::permissions::control(&db, "*", true, "fixture", now).unwrap();
        }
        core.record_store(StorePlatform::Android, "15.8.0", true, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 3);
        {
            let db = core.database.lock().unwrap();
            crate::permissions::control(&db, "*", false, "fixture", now).unwrap();
        }
        core.record_store(StorePlatform::Android, "15.8.0", true, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 3);
        {
            let mut db = core.database.lock().unwrap();
            let tx = db.transaction().unwrap();
            let event: CoreEvent = serde_json::from_value(serde_json::json!({"type":"messageReceived","eventId":"setting","chatId":"m1","messageId":"1","text":"","createdAtMs":now})).unwrap();
            configure(&core, &tx, &event, &["android,ios".into(), "off".into()]).unwrap();
            tx.commit().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM store_subscriptions WHERE chat='m1'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
        assert_eq!(core.stats().unwrap().queued_actions, 1);
        core.record_store(StorePlatform::Ios, "15.8.0", true, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 1);
        drop(core);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn outbox_capacity_keeps_previous_version() {
        let directory = std::env::temp_dir().join(format!(
            "kbc-store-capacity-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let core = runtime(&directory.join("core.sqlite"));
        let now = now_ms();
        {
            let db = core.database.lock().unwrap();
            db.execute("INSERT INTO store_subscriptions VALUES('android','m1')", [])
                .unwrap();
        }
        core.record_store(StorePlatform::Android, "15.6.0", false, now)
            .unwrap();
        {
            let mut db = core.database.lock().unwrap();
            let tx = db.transaction().unwrap();
            for index in 0..crate::MAX_ACTIONS {
                tx.execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,'test','m2','',?2,?2,'unknown')",params![format!("test-{index}"),now]).unwrap();
            }
            tx.commit().unwrap();
        }
        assert!(
            core.record_store(StorePlatform::Android, "15.7.0", true, now)
                .is_err()
        );
        {
            let db = core.database.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT version FROM store_versions WHERE platform='android'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
                "15.6.0"
            );
            db.execute("DELETE FROM actions WHERE id='test-0'", [])
                .unwrap();
        }
        core.record_store(StorePlatform::Android, "15.7.0", true, now)
            .unwrap();
        assert_eq!(core.stats().unwrap().queued_actions, 1);
        drop(core);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    #[ignore = "公開ストアへ接続する単発の移植確認"]
    async fn public_sources() {
        let http = Arc::new(crate::assets::AssetService::new("main").unwrap());
        for platform in [StorePlatform::Android, StorePlatform::Ios] {
            let version = StoreVersionSource::new(platform, Arc::clone(&http))
                .fetch()
                .await
                .unwrap();
            println!("{}: {}", platform.key(), version);
        }
    }
}
