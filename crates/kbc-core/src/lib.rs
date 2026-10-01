use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kbc_protocol::*;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Notify;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const MAX_EVENTS: i64 = 8192;
const MAX_ACTIONS: i64 = 2048;
const RETENTION_MS: i64 = 48 * 60 * 60 * 1000;

pub struct Runtime {
    database: Mutex<Connection>,
    wake: Notify,
    stopped: AtomicBool,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl Runtime {
    pub fn open(config: CoreConfig) -> Result<Self> {
        if config.owner_id.is_empty() {
            return Err("MissingOwner".into());
        }
        if let Some(parent) = Path::new(&config.database_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(config.database_path)?;
        // Event・Action・checkpointを一つのtransactionで確定する。
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            PRAGMA max_page_count=16384;
            CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS checkpoints (stream TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS events (id TEXT PRIMARY KEY, payload TEXT NOT NULL, received INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS event_received ON events(received);
            CREATE TABLE IF NOT EXISTS actions (
                id TEXT PRIMARY KEY, event_id TEXT NOT NULL, chat TEXT NOT NULL,
                payload TEXT NOT NULL, due INTEGER NOT NULL, created INTEGER NOT NULL,
                status TEXT NOT NULL, code TEXT NOT NULL DEFAULT '', completed INTEGER);
            CREATE INDEX IF NOT EXISTS action_due ON actions(status, due);")?;
        let owner: Option<String> = db
            .query_row("SELECT value FROM metadata WHERE key='owner'", [], |row| {
                row.get(0)
            })
            .optional()?;
        if owner
            .as_ref()
            .is_some_and(|owner| owner != &config.owner_id)
        {
            return Err("OwnerMismatch".into());
        }
        db.execute(
            "INSERT OR IGNORE INTO metadata VALUES ('owner', ?1)",
            [&config.owner_id],
        )?;
        // 通信開始済みの操作は、再起動後に勝手に再投稿しない。
        db.execute("UPDATE actions SET status='unknown', code='RestartDuringSend', completed=?1 WHERE status='sending'", [now_ms()])?;
        Ok(Self {
            database: Mutex::new(db),
            wake: Notify::new(),
            stopped: AtomicBool::new(false),
        })
    }

    pub fn checkpoint(&self, stream: &str) -> Result<Option<String>> {
        Ok(self
            .database
            .lock()
            .map_err(|_| "DatabaseLock")?
            .query_row(
                "SELECT value FROM checkpoints WHERE stream=?1",
                [stream],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn submit_batch(&self, batch: ReceivedBatch) -> Result<BatchReceipt> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("Stopped".into());
        }
        if batch.protocol_version != PROTOCOL_VERSION {
            return Err("ProtocolMismatch".into());
        }
        if batch.events.len() > 100 || batch.checkpoint.len() > 8192 || batch.stream_key.len() > 256
        {
            return Err("BatchLimit".into());
        }
        let now = now_ms();
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        // 結果不明・送信待ちの記録は容量確保のために削除しない。
        tx.execute("DELETE FROM events WHERE received < ?1 AND NOT EXISTS
            (SELECT 1 FROM actions WHERE event_id=events.id AND status IN ('queued','sending','unknown','failed'))", [now - RETENTION_MS])?;
        tx.execute(
            "DELETE FROM actions WHERE status='sent' AND completed < ?1",
            [now - RETENTION_MS],
        )?;
        let mut receipt = BatchReceipt {
            accepted: 0,
            duplicates: 0,
            actions_created: 0,
        };
        let mut bytes = 0;
        for event in batch.events {
            let payload = serde_json::to_string(&event)?;
            bytes += payload.len();
            if bytes > 256 * 1024 {
                return Err("BatchByteLimit".into());
            }
            let CoreEvent::MessageReceived {
                event_id,
                chat_id,
                message_id,
                text,
                created_at_ms,
            } = &event;
            if [event_id, chat_id, message_id]
                .iter()
                .any(|id| id.is_empty() || id.len() > 256)
                || text.len() > 32 * 1024
                || *created_at_ms < 0
            {
                return Err("InvalidEvent".into());
            }
            if tx.execute(
                "INSERT OR IGNORE INTO events VALUES (?1,?2,?3)",
                params![event_id, payload, now],
            )? == 0
            {
                receipt.duplicates += 1;
                continue;
            }
            receipt.accepted += 1;
            if batch
                .baseline_before_ms
                .is_some_and(|before| *created_at_ms < before)
            {
                continue;
            }
            // この段階は小さな疎通Commandのみ。HTTP等をtransaction内で待たない。
            let responses = command_responses(text, now);
            for (index, (body, due)) in responses.into_iter().enumerate() {
                let id = format!("{event_id}:{index}");
                let action = CoreAction::SendMessage {
                    action_id: id.clone(),
                    event_id: event_id.clone(),
                    chat_id: chat_id.clone(),
                    related_message_id: message_id.clone(),
                    text: body,
                    created_at_ms: now,
                };
                tx.execute(
                    "INSERT INTO actions (id,event_id,chat,payload,due,created,status)
                    VALUES (?1,?2,?3,?4,?5,?6,'queued')",
                    params![
                        id,
                        event_id,
                        chat_id,
                        serde_json::to_string(&action)?,
                        due,
                        now
                    ],
                )?;
                receipt.actions_created += 1;
            }
        }
        let event_count: i64 = tx.query_row("SELECT count(*) FROM events", [], |row| row.get(0))?;
        let action_count: i64 =
            tx.query_row("SELECT count(*) FROM actions", [], |row| row.get(0))?;
        if event_count > MAX_EVENTS || action_count > MAX_ACTIONS {
            return Err("StoreCapacity".into());
        }
        tx.execute("INSERT INTO checkpoints VALUES (?1,?2) ON CONFLICT(stream) DO UPDATE SET value=excluded.value",
            params![batch.stream_key, batch.checkpoint])?;
        tx.commit()?;
        drop(db);
        self.wake.notify_waiters();
        Ok(receipt)
    }

    pub async fn next_action(&self) -> Result<Option<CoreAction>> {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.stopped.load(Ordering::Acquire) {
                return Ok(None);
            }
            let delay = {
                let db = self.database.lock().map_err(|_| "DatabaseLock")?;
                let next: Option<(String, String, i64)> = db.query_row(
                    "SELECT a.id,a.payload,a.due FROM actions a WHERE a.status='queued'
                    AND NOT EXISTS (SELECT 1 FROM actions b WHERE b.chat=a.chat AND b.status='sending')
                    ORDER BY a.due,a.rowid LIMIT 1", [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
                match next {
                    Some((id, payload, due)) if due <= now_ms() => {
                        let action = serde_json::from_str(&payload)?;
                        db.execute("UPDATE actions SET status='sending' WHERE id=?1", [id])?;
                        return Ok(Some(action));
                    }
                    Some((_, _, due)) => {
                        Some(Duration::from_millis((due - now_ms()).max(1) as u64))
                    }
                    None => None,
                }
            };
            if let Some(delay) = delay {
                tokio::select! { _ = notified => {}, _ = tokio::time::sleep(delay) => {} }
            } else {
                notified.await;
            }
        }
    }

    pub fn complete_action(&self, result: ActionResult) -> Result<()> {
        let status = match result.status {
            DeliveryStatus::Sent => "sent",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::Unknown => "unknown",
        };
        if result.code.len() > 80 {
            return Err("InvalidResult".into());
        }
        let count = self.database.lock().map_err(|_| "DatabaseLock")?.execute(
            "UPDATE actions SET status=?2, code=?3, completed=?4 WHERE id=?1 AND status='sending'",
            params![result.action_id, status, result.code, now_ms()],
        )?;
        if count != 1 {
            return Err("ActionNotSending".into());
        }
        self.wake.notify_waiters();
        Ok(())
    }

    pub fn stats(&self) -> Result<CoreStats> {
        let db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let count = |status| -> Result<u32> {
            Ok(db.query_row(
                "SELECT count(*) FROM actions WHERE status=?1",
                [status],
                |row| row.get(0),
            )?)
        };
        Ok(CoreStats {
            retained_events: db.query_row("SELECT count(*) FROM events", [], |row| row.get(0))?,
            queued_actions: count("queued")?,
            sending_actions: count("sending")?,
            unknown_actions: count("unknown")?,
            failed_actions: count("failed")?,
        })
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
}

fn command_responses(text: &str, now: i64) -> Vec<(String, i64)> {
    match text {
        "o.ping" => vec![("pong!".into(), now)],
        "o.ping help" => vec![(
            "o.ping\nbotが反応できる状態か確認します。pong! と返れば正常です。".into(),
            now,
        )],
        _ => {
            let Some(argument) = text.strip_prefix("o.test-notify ") else {
                return Vec::new();
            };
            match argument.parse::<i64>() {
                Ok(seconds @ 1..=60) => vec![
                    (format!("{seconds}秒後に通知を送ります。"), now),
                    (
                        "通知の確認です。次の入力がなくても送信されます。".into(),
                        now + seconds * 1000,
                    ),
                ],
                _ => vec![("確認用: o.test-notify 1〜60（秒）".into(), now)],
            }
        }
    }
}
