use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use kbc_protocol::*;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Notify;

mod commands;
mod images;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const DEFAULT_MAX_EVENTS: u32 = 131072;
const MAX_ACTIONS: i64 = 2048;
const MAX_COMPLETED_ACTIONS: i64 = 2048;
const RETENTION_MS: i64 = 48 * 60 * 60 * 1000;

pub struct Runtime {
    database: Mutex<Connection>,
    wake: Notify,
    stopped: AtomicBool,
    next_cleanup_ms: AtomicI64,
    max_retained_events: u32,
    content: commands::content::ContentCatalog,
    search: commands::search::SearchCatalog,
    images: images::ImageService,
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
        let max_retained_events = config.max_retained_events.unwrap_or(DEFAULT_MAX_EVENTS);
        if !(8192..=524288).contains(&max_retained_events) {
            return Err("InvalidEventCapacity".into());
        }
        let content = commands::content::ContentCatalog::load(Path::new(
            config.content_directory.as_deref().unwrap_or("content"),
        ))?;
        let search = commands::search::SearchCatalog::load(Path::new(
            config
                .search_data_path
                .as_deref()
                .unwrap_or("data/search/catalog.json"),
        ))?;
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
            CREATE INDEX IF NOT EXISTS action_due ON actions(status, due);
            CREATE INDEX IF NOT EXISTS action_event ON actions(event_id,status);
            CREATE INDEX IF NOT EXISTS action_completed ON actions(completed)
                WHERE status IN ('sent','failed');
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,chat TEXT NOT NULL,owner TEXT NOT NULL,
                action TEXT NOT NULL UNIQUE,prompt TEXT,payload TEXT NOT NULL,
                expires INTEGER NOT NULL,revision TEXT NOT NULL,UNIQUE(chat,owner));
            CREATE INDEX IF NOT EXISTS session_expiry ON sessions(expires);
            CREATE INDEX IF NOT EXISTS session_prompt ON sessions(chat,owner,prompt);")?;
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
        // 取り出しただけの操作は再待機、通信開始済みは勝手に再投稿しない。
        db.execute(
            "UPDATE actions SET status='queued' WHERE status='claimed'",
            [],
        )?;
        db.execute("UPDATE actions SET status='unknown', code='RestartDuringSend', completed=?1 WHERE status='sending'", [now_ms()])?;
        // Commandは受付でActionへ変換済み。本文は長期ログではない。
        db.execute("UPDATE events SET payload='' WHERE payload<>''", [])?;
        db.execute(
            "UPDATE actions SET payload='' WHERE status IN ('sent','failed')",
            [],
        )?;
        db.execute(
            "DELETE FROM sessions WHERE expires<=?1 OR revision<>?2",
            params![now_ms(), search.revision],
        )?;
        Ok(Self {
            database: Mutex::new(db),
            wake: Notify::new(),
            stopped: AtomicBool::new(false),
            next_cleanup_ms: AtomicI64::new(0),
            max_retained_events,
            content,
            search,
            images: images::ImageService::new()?,
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
        if batch.events.len() > 100
            || batch.checkpoint.len() > 64 * 1024
            || batch.stream_key.len() > 256
        {
            return Err("BatchLimit".into());
        }
        let now = now_ms();
        let mut bytes = 0;
        let mut plans = Vec::with_capacity(batch.events.len());
        // 検索は不変snapshotから作り、SQLiteのlockを持って走査しない。
        for event in &batch.events {
            bytes += serde_json::to_string(event)?.len();
            if bytes > 256 * 1024 {
                return Err("BatchByteLimit".into());
            }
            let CoreEvent::MessageReceived {
                event_id,
                chat_id,
                message_id,
                text,
                sender_id,
                reply_to_message_id,
                created_at_ms,
            } = event;
            if [event_id, chat_id, message_id]
                .iter()
                .any(|id| id.is_empty() || id.len() > 256)
                || sender_id
                    .iter()
                    .chain(reply_to_message_id.iter())
                    .any(|id| id.is_empty() || id.len() > 256)
                || text.len() > 32 * 1024
                || *created_at_ms < 0
            {
                return Err("InvalidEvent".into());
            }
            plans.push(
                if batch
                    .baseline_before_ms
                    .is_some_and(|before| *created_at_ms < before)
                {
                    commands::CommandPlan::Ignore
                } else {
                    commands::prepare(&self.content, &self.search, text, now)
                },
            );
        }
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        let cleanup_due = now >= self.next_cleanup_ms.load(Ordering::Relaxed);
        tx.execute("DELETE FROM sessions WHERE expires<=?1", [now])?;
        if cleanup_due {
            // 未解決操作のIDは残す。頻繁な空checkpoint保存で全行を掃除しない。
            tx.execute("DELETE FROM events WHERE received < ?1 AND NOT EXISTS
                (SELECT 1 FROM actions WHERE event_id=events.id AND status IN ('queued','claimed','sending','unknown'))", [now - RETENTION_MS])?;
            tx.execute(
                "DELETE FROM actions WHERE status IN ('sent','failed') AND completed < ?1",
                [now - RETENTION_MS],
            )?;
        }
        let mut receipt = BatchReceipt {
            accepted: 0,
            duplicates: 0,
            actions_created: 0,
        };
        for (event, plan) in batch.events.into_iter().zip(plans) {
            let CoreEvent::MessageReceived {
                event_id,
                chat_id,
                message_id,
                created_at_ms,
                ..
            } = &event;
            if tx.execute(
                "INSERT OR IGNORE INTO events VALUES (?1,'',?2)",
                params![event_id, now],
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
            let (responses, replacement) =
                commands::sessions::apply(&tx, &event, plan, &self.search, now)?;
            let responses = commands::split_responses(responses)?;
            for (index, (body, due, image_url)) in responses.into_iter().enumerate() {
                let id = format!("{event_id}:{index}");
                let is_prompt: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE action=?1)",
                    [&id],
                    |row| row.get(0),
                )?;
                let action = CoreAction::SendMessage {
                    action_id: id.clone(),
                    event_id: event_id.clone(),
                    chat_id: chat_id.clone(),
                    related_message_id: message_id.clone(),
                    text: body,
                    image_url,
                    replace_message_id: if index == 0 {
                        replacement.clone()
                    } else {
                        None
                    },
                    is_prompt,
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
        let action_count: i64 = tx.query_row(
            "SELECT count(*) FROM actions WHERE status IN ('queued','claimed','sending','unknown')",
            [],
            |row| row.get(0),
        )?;
        if event_count > i64::from(self.max_retained_events) || action_count > MAX_ACTIONS {
            return Err("StoreCapacity".into());
        }
        tx.execute("INSERT INTO checkpoints VALUES (?1,?2) ON CONFLICT(stream) DO UPDATE SET value=excluded.value",
            params![batch.stream_key, batch.checkpoint])?;
        tx.commit()?;
        if cleanup_due {
            self.next_cleanup_ms.store(now + 60000, Ordering::Relaxed);
        }
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
                    AND NOT EXISTS (SELECT 1 FROM actions b WHERE b.chat=a.chat AND b.status IN ('claimed','sending'))
                    ORDER BY a.due,a.rowid LIMIT 1", [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
                match next {
                    Some((id, payload, due)) if due <= now_ms() => {
                        let action = serde_json::from_str(&payload)?;
                        db.execute("UPDATE actions SET status='claimed' WHERE id=?1", [id])?;
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

    pub fn mark_sending(&self, action_id: &str) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("Stopped".into());
        }
        let count = self.database.lock().map_err(|_| "DatabaseLock")?.execute(
            "UPDATE actions SET status='sending' WHERE id=?1 AND status='claimed'",
            [action_id],
        )?;
        if count != 1 {
            return Err("ActionNotClaimed".into());
        }
        Ok(())
    }

    pub fn retry_action(&self, action_id: &str, delay_ms: u32) -> Result<()> {
        if delay_ms > 600000 {
            return Err("InvalidRetryDelay".into());
        }
        let count = self.database.lock().map_err(|_| "DatabaseLock")?.execute(
            "UPDATE actions SET status='queued', due=MAX(due,?2) WHERE id=?1 AND status='claimed'",
            params![action_id, now_ms() + i64::from(delay_ms)],
        )?;
        if count != 1 {
            return Err("ActionNotClaimed".into());
        }
        self.wake.notify_waiters();
        Ok(())
    }

    pub fn complete_action(&self, result: ActionResult) -> Result<()> {
        self.finish_action(result, "sending")
    }

    // 運用者が照合した結果だけを明示的に確定する。結果不明の自動再送には使わない。
    pub fn resolve_action(&self, result: ActionResult) -> Result<()> {
        if matches!(result.status, DeliveryStatus::Unknown) {
            return Err("InvalidResolution".into());
        }
        self.finish_action(result, "unknown")
    }

    fn finish_action(&self, result: ActionResult, previous: &str) -> Result<()> {
        let status = match result.status {
            DeliveryStatus::Sent => "sent",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::Unknown => "unknown",
        };
        if result.code.len() > 80 {
            return Err("InvalidResult".into());
        }
        if result
            .message_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        {
            return Err("InvalidMessageId".into());
        }
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        let payload: String = tx
            .query_row(
                "SELECT payload FROM actions WHERE id=?1 AND status=?2",
                params![result.action_id, previous],
                |row| row.get(0),
            )
            .optional()?
            .ok_or("InvalidActionState")?;
        let completed: CoreAction = serde_json::from_str(&payload)?;
        tx.execute(
            "UPDATE actions SET status=?2,code=?3,completed=?4,
            payload=CASE WHEN ?2 IN ('sent','failed') THEN '' ELSE payload END WHERE id=?1",
            params![result.action_id, status, result.code, now_ms()],
        )?;
        if matches!(result.status, DeliveryStatus::Sent) {
            let waiting: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE action=?1 AND expires>?2)",
                params![result.action_id, now_ms()],
                |row| row.get(0),
            )?;
            if waiting && result.message_id.is_none() {
                return Err("MissingPromptMessageId".into());
            }
            tx.execute(
                "UPDATE sessions SET prompt=?2,expires=?3 WHERE action=?1 AND expires>?4",
                params![
                    result.action_id,
                    result.message_id,
                    now_ms() + commands::search::SESSION_TTL_MS,
                    now_ms()
                ],
            )?;
            if let CoreAction::SendMessage {
                event_id,
                chat_id,
                replace_message_id,
                is_prompt,
                ..
            } = &completed
            {
                if let Some(old) = replace_message_id {
                    enqueue_cleanup(&tx, event_id, chat_id, old, now_ms())?;
                }
                if *is_prompt && let Some(message_id) = &result.message_id {
                    enqueue_cleanup(
                        &tx,
                        event_id,
                        chat_id,
                        message_id,
                        now_ms() + commands::search::SESSION_TTL_MS,
                    )?;
                }
            }
            if let CoreAction::DeleteMessage {
                chat_id,
                message_id,
                ..
            } = &completed
            {
                tx.execute(
                    "DELETE FROM sessions WHERE chat=?1 AND prompt=?2",
                    params![chat_id, message_id],
                )?;
            }
        } else if matches!(result.status, DeliveryStatus::Failed) {
            tx.execute("DELETE FROM sessions WHERE action=?1", [&result.action_id])?;
        }
        tx.execute(
            "DELETE FROM actions WHERE id IN
            (SELECT id FROM actions WHERE status IN ('sent','failed')
            ORDER BY completed DESC,rowid DESC LIMIT -1 OFFSET ?1)",
            [MAX_COMPLETED_ACTIONS],
        )?;
        tx.commit()?;
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
            max_retained_events: self.max_retained_events,
            queued_actions: count("queued")?,
            claimed_actions: count("claimed")?,
            sending_actions: count("sending")?,
            unknown_actions: count("unknown")?,
            failed_actions: count("failed")?,
            completed_actions: count("sent")? + count("failed")?,
            active_sessions: db.query_row(
                "SELECT count(*) FROM sessions WHERE expires>?1",
                [now_ms()],
                |row| row.get(0),
            )?,
        })
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }

    pub async fn prepare_image(&self, action_id: &str) -> Result<Option<Vec<u8>>> {
        let mut action: CoreAction = {
            let db = self.database.lock().map_err(|_| "DatabaseLock")?;
            let payload: String = db.query_row(
                "SELECT payload FROM actions WHERE id=?1 AND status='claimed'",
                [action_id],
                |row| row.get(0),
            )?;
            serde_json::from_str(&payload)?
        };
        let CoreAction::SendMessage {
            image_url, text, ..
        } = &mut action
        else {
            return Err("NotImageAction".into());
        };
        let url = image_url.as_deref().ok_or("NotImageAction")?;
        match self.images.download(url).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => {
                // 取得失敗はLINEへ何も送っていない。元URLを示す通常返信へ切り替える。
                *text = format!("画像を取得できませんでした。こちらから確認してください。\n{url}");
                *image_url = None;
                let count = self.database.lock().map_err(|_| "DatabaseLock")?.execute("UPDATE actions SET payload=?2,status='queued',code='ImageUnavailable' WHERE id=?1 AND status='claimed'", params![action_id, serde_json::to_string(&action)?])?;
                if count != 1 {
                    return Err("ActionNotClaimed".into());
                }
                self.wake.notify_waiters();
                Ok(None)
            }
        }
    }
}

fn enqueue_cleanup(
    tx: &rusqlite::Transaction<'_>,
    event_id: &str,
    chat_id: &str,
    message_id: &str,
    due: i64,
) -> Result<()> {
    let id = format!("cleanup:{}", serde_json::to_string(&(chat_id, message_id))?);
    let action = CoreAction::DeleteMessage {
        action_id: id.clone(),
        event_id: event_id.into(),
        chat_id: chat_id.into(),
        message_id: message_id.into(),
        created_at_ms: now_ms(),
    };
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM actions WHERE id=?1)",
        [&id],
        |row| row.get(0),
    )?;
    if !exists {
        let retained: i64 = tx.query_row(
            "SELECT count(*) FROM actions WHERE status IN ('queued','claimed','sending','unknown')",
            [],
            |row| row.get(0),
        )?;
        if retained >= MAX_ACTIONS {
            return Ok(());
        }
    }
    // 同じpromptの期限清掃を前倒しする。重複した取消しAPIを増やさない。
    tx.execute("INSERT INTO actions (id,event_id,chat,payload,due,created,status) VALUES (?1,?2,?3,?4,?5,?6,'queued')
        ON CONFLICT(id) DO UPDATE SET due=MIN(actions.due,excluded.due) WHERE actions.status='queued'", params![id, event_id, chat_id, serde_json::to_string(&action)?, due, now_ms()])?;
    Ok(())
}
