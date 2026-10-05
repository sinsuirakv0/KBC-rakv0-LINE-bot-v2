//! 公開ハッシュ確認、履歴の追跡、通知先の有限配送を既存監視workerへ接続する。

use super::{pending_count, root_action, schedule_contents, source::SkdDataSource};
use crate::{Result, Runtime, now_ms};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const STATE_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/state/skd-notifications.json";
const MAX_PENDING_UPDATES: i64 = 16;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceState {
    version: u32,
    last_hashes: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
struct Watermark {
    timestamp: i64,
    hashes: BTreeMap<String, String>,
}

pub(crate) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS schedule_updates(timestamp INTEGER PRIMARY KEY,contents TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS schedule_targets(timestamp INTEGER NOT NULL,chat TEXT NOT NULL,PRIMARY KEY(timestamp,chat));")?;
    Ok(())
}
pub(crate) fn display_watermark(value: &str) -> Option<String> {
    let watermark: Watermark = serde_json::from_str(value).ok()?;
    crate::store_update::detected_at(watermark.timestamp.checked_mul(1000)?).ok()
}
impl Runtime {
    pub(crate) async fn schedule_loop(&self) {
        let mut failures = 0usize;
        loop {
            let result = tokio::select! {
                _ = self.cancel.cancelled() => return,
                result = self.poll_schedule() => result,
            };
            let delay = match result {
                Ok(()) => {
                    failures = 0;
                    60
                }
                Err(error) => {
                    eprintln!("Schedule monitor: {error}");
                    if let Ok(db) = self.database.lock() {
                        let _ = db.execute("INSERT INTO store_versions VALUES('skd','',0,'ScheduleCheckFailed') ON CONFLICT(platform) DO UPDATE SET error=excluded.error WHERE error<>excluded.error",[]);
                    }
                    let delay = [60, 120, 300][failures.min(2)];
                    failures = failures.saturating_add(1);
                    delay
                }
            };
            tokio::select! { _=self.cancel.cancelled()=>return, _=tokio::time::sleep(Duration::from_secs(delay))=>{} }
        }
    }
    async fn schedule_hashes(&self) -> Result<BTreeMap<String, String>> {
        parse_hashes(&self.assets.get_text(STATE_URL).await?)
    }
    async fn poll_schedule(&self) -> Result<()> {
        self.dispatch_schedules()?;
        let previous: Option<String> = {
            let db = self.database.lock().map_err(|_| "DatabaseLock")?;
            if !db.query_row(
                "SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform='skd')",
                [],
                |row| row.get::<_, bool>(0),
            )? {
                return Ok(());
            }
            if db.query_row("SELECT count(*) FROM schedule_updates", [], |row| {
                row.get::<_, i64>(0)
            })? >= MAX_PENDING_UPDATES
            {
                return Err("ScheduleQueueFull".into());
            }
            db.query_row(
                "SELECT NULLIF(version,'') FROM store_versions WHERE platform='skd'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
        };
        let hashes = self.schedule_hashes().await?;
        let previous: Option<Watermark> =
            previous.as_deref().map(serde_json::from_str).transpose()?;
        let changed = previous.as_ref().is_none_or(|old| old.hashes != hashes);
        let source = SkdDataSource::new(
            Arc::new(self.assets.clone()),
            Arc::clone(&self.content.messages),
        );
        let (watermark, updates) = if let Some(previous) = previous {
            if previous.hashes == hashes {
                (previous, Vec::new())
            } else {
                let (updates, more) = source.updates_since(previous.timestamp, &hashes).await?;
                let watermark = Watermark {
                    timestamp: updates
                        .last()
                        .map_or(previous.timestamp, |update| update.cursor),
                    hashes: if more {
                        BTreeMap::new()
                    } else {
                        hashes.clone()
                    },
                };
                let updates = updates
                    .into_iter()
                    .map(|update| {
                        let timestamp = update.cursor;
                        Ok((timestamp, schedule_contents(self, update)?))
                    })
                    .collect::<Result<Vec<_>>>()?;
                (watermark, updates)
            }
        } else {
            (
                Watermark {
                    timestamp: source.latest_timestamp(&hashes).await?,
                    hashes: hashes.clone(),
                },
                Vec::new(),
            )
        };
        if changed && self.schedule_hashes().await? != hashes {
            return Err("ScheduleSourceChanged".into());
        }
        self.observe_schedule(watermark, updates)?;
        self.dispatch_schedules()
    }
    fn observe_schedule(
        &self,
        watermark: Watermark,
        updates: Vec<(i64, Vec<String>)>,
    ) -> Result<()> {
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform='skd')",
            [],
            |row| row.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM store_subscriptions WHERE platform='skd' AND NOT EXISTS(SELECT 1 FROM bot_stops WHERE bot_stops.chat IN ('*',store_subscriptions.chat)))",[],|row|row.get(0))?;
        let updates = if active { updates } else { Vec::new() };
        // 配信先のない更新は残さず、停止中も検知位置だけは進める。
        tx.execute("DELETE FROM schedule_updates WHERE NOT EXISTS(SELECT 1 FROM schedule_targets t WHERE t.timestamp=schedule_updates.timestamp)",[])?;
        let count: i64 = tx.query_row("SELECT count(*) FROM schedule_updates", [], |row| {
            row.get(0)
        })?;
        if count + updates.len() as i64 > MAX_PENDING_UPDATES {
            return Err("ScheduleQueueFull".into());
        }
        for (timestamp, contents) in updates {
            tx.execute(
                "INSERT OR IGNORE INTO schedule_updates VALUES(?1,?2)",
                params![timestamp, serde_json::to_string(&contents)?],
            )?;
            tx.execute("INSERT OR IGNORE INTO schedule_targets SELECT ?1,chat FROM store_subscriptions WHERE platform='skd' AND NOT EXISTS(SELECT 1 FROM bot_stops WHERE bot_stops.chat IN ('*',store_subscriptions.chat))",[timestamp])?;
        }
        tx.execute("DELETE FROM schedule_updates WHERE NOT EXISTS(SELECT 1 FROM schedule_targets t WHERE t.timestamp=schedule_updates.timestamp)",[])?;
        tx.execute("INSERT INTO store_versions VALUES('skd',?1,?2,'') ON CONFLICT(platform) DO UPDATE SET version=excluded.version,checked=excluded.checked,error=''",params![serde_json::to_string(&watermark)?,now_ms()])?;
        tx.commit()?;
        Ok(())
    }
    fn dispatch_schedules(&self) -> Result<()> {
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        let targets = {
            let mut statement=tx.prepare("SELECT t.timestamp,t.chat,u.contents FROM schedule_targets t JOIN schedule_updates u USING(timestamp) ORDER BY timestamp,chat LIMIT 64")?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if targets.is_empty() {
            return Ok(());
        }
        let mut pending = pending_count(&tx)?;
        for (timestamp, chat, body) in targets {
            if crate::permissions::stopped(&tx, &chat)? {
                tx.execute(
                    "DELETE FROM schedule_targets WHERE timestamp=?1 AND chat=?2",
                    params![timestamp, chat],
                )?;
                continue;
            }
            let contents: Vec<String> = serde_json::from_str(&body)?;
            let reserved = contents.len() as i64 + 1;
            if pending + reserved > crate::MAX_ACTIONS {
                break;
            }
            let event = format!("store:skd:{timestamp}:{chat}");
            let id = format!("{event}:0");
            let now = now_ms();
            let action = root_action(self, id.clone(), event.clone(), chat.clone(), contents, now)?;
            let inserted=tx.execute("INSERT OR IGNORE INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,?5,?5,'queued')",params![id,event,chat,serde_json::to_string(&action)?,now])?;
            tx.execute(
                "DELETE FROM schedule_targets WHERE timestamp=?1 AND chat=?2",
                params![timestamp, chat],
            )?;
            pending += reserved * inserted as i64;
        }
        tx.execute("DELETE FROM schedule_updates WHERE NOT EXISTS(SELECT 1 FROM schedule_targets t WHERE t.timestamp=schedule_updates.timestamp)",[])?;
        tx.commit()?;
        self.wake.notify_waiters();
        Ok(())
    }
}

pub(super) fn parse_hashes(text: &str) -> Result<BTreeMap<String, String>> {
    let state: SourceState = serde_json::from_str(text)?;
    if state.version != 1
        || state.last_hashes.len() != 3
        || ["gatya", "sale", "item"].iter().any(|kind| {
            state
                .last_hashes
                .get(*kind)
                .is_none_or(|hash| hash.len() != 32 || !hash.bytes().all(|c| c.is_ascii_hexdigit()))
        })
    {
        return Err("InvalidScheduleState".into());
    }
    Ok(state.last_hashes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skd::tests::{fixture, open};

    #[test]
    fn persists_fanout_and_resumes_with_bounded_outbox() {
        let (path, runtime) = fixture();
        {
            let db = runtime.database.lock().unwrap();
            for index in 0..100 {
                db.execute(
                    "INSERT INTO store_subscriptions VALUES('skd',?1)",
                    [format!("chat-{index}")],
                )
                .unwrap();
            }
        }
        runtime
            .observe_schedule(
                Watermark {
                    timestamp: 100,
                    hashes: BTreeMap::new(),
                },
                Vec::new(),
            )
            .unwrap();
        assert_eq!(
            runtime
                .database
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM actions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        runtime
            .observe_schedule(
                Watermark {
                    timestamp: 200,
                    hashes: BTreeMap::new(),
                },
                vec![(
                    200,
                    vec!["スレッド本文".into(); super::super::MAX_THREAD_MESSAGES],
                )],
            )
            .unwrap();
        runtime.dispatch_schedules().unwrap();
        let remaining: i64 = runtime
            .database
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM schedule_targets", [], |r| r.get(0))
            .unwrap();
        assert!(remaining > 0 && remaining < 100);
        assert!(pending_count(&runtime.database.lock().unwrap()).unwrap() <= crate::MAX_ACTIONS);
        runtime.shutdown();
        drop(runtime);
        let runtime = open(&path);
        {
            let db = runtime.database.lock().unwrap();
            db.execute(
                "UPDATE actions SET status='sent',payload='',completed=0",
                [],
            )
            .unwrap();
        }
        runtime.dispatch_schedules().unwrap();
        assert_eq!(
            runtime
                .database
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM schedule_targets", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            runtime
                .database
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM actions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            100
        );
        runtime.dispatch_schedules().unwrap();
        assert_eq!(
            runtime
                .database
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM actions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            100
        );
        // 停止先の予定は見送り、再開後に遅れて送信しない。
        {
            let db = runtime.database.lock().unwrap();
            crate::permissions::control(&db, "*", true, "fixture", now_ms()).unwrap();
        }
        runtime
            .observe_schedule(
                Watermark {
                    timestamp: 201,
                    hashes: BTreeMap::new(),
                },
                vec![(201, vec!["停止中".into()])],
            )
            .unwrap();
        runtime.dispatch_schedules().unwrap();
        {
            let db = runtime.database.lock().unwrap();
            assert_eq!(
                db.query_row("SELECT count(*) FROM schedule_targets", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM actions", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                100
            );
            crate::permissions::control(&db, "*", false, "fixture", now_ms()).unwrap();
        }
        {
            let db = runtime.database.lock().unwrap();
            for timestamp in 201..217 {
                db.execute("INSERT INTO schedule_updates VALUES(?1,'[]')", [timestamp])
                    .unwrap();
                db.execute(
                    "INSERT INTO schedule_targets VALUES(?1,'chat-0')",
                    [timestamp],
                )
                .unwrap();
            }
        }
        assert!(
            runtime
                .observe_schedule(
                    Watermark {
                        timestamp: 217,
                        hashes: BTreeMap::new()
                    },
                    vec![(217, vec!["次".into()])]
                )
                .is_err()
        );
        let value: String = runtime
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT version FROM store_versions WHERE platform='skd'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Watermark>(&value).unwrap().timestamp,
            201
        );
        drop(runtime);
        std::fs::remove_dir_all(path).unwrap();
    }
}
