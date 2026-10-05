use crate::messages::message;
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

mod assets;
mod commands;
mod logs;
mod media;
mod messages;
mod motion;
mod oc;
mod permissions;
mod store_update;

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
    permissions: permissions::Permissions,
    content: commands::content::ContentCatalog,
    search_path: std::path::PathBuf,
    search_live: bool,
    assets: assets::AssetService,
    media_root: std::path::PathBuf,
    ffmpeg_path: Option<std::path::PathBuf>,
    cancel: tokio_util::sync::CancellationToken,
    media_worker_active: AtomicBool,
    logs_enabled: bool,
    store_worker_active: AtomicBool,
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
        let search_path = config
            .search_data_path
            .as_deref()
            .unwrap_or("data/search/catalog.json")
            .into();
        if let Some(parent) = Path::new(&config.database_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let media_root = Path::new(&config.database_path)
            .parent()
            .unwrap_or(Path::new("storage"))
            .join("media");
        std::fs::create_dir_all(&media_root)?;
        let permissions = permissions::Permissions::load(config.permissions_path.as_deref())?;
        let mut db = Connection::open(config.database_path)?;
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
        oc::initialize(&db)?;
        commands::sessions::initialize(&db)?;
        logs::initialize(&db)?;
        store_update::initialize(&db)?;
        oc::import_legacy(&mut db, config.legacy_oc_settings_path.as_deref())?;
        // 遠隔退避以後に送信された可能性がある。期限済みの副作用は照合まで再実行しない。
        if config.restored_from_backup.unwrap_or(false) {
            db.execute("UPDATE actions SET status='unknown',code='BackupRestoreNeedsReconciliation',completed=?1
                WHERE status IN ('queued','claimed','sending') AND due<=?1 AND (json_extract(payload,'$.type') IN ('sendMessage','deleteMessage')
                OR (json_extract(payload,'$.type')='ocApi' AND json_extract(payload,'$.request.type') IN ('membership','report')))",[now_ms()])?;
        }
        // 読み取りだけのOC照会は再実行できる。更新の結果不明と区別する。
        db.execute("UPDATE actions SET status='queued' WHERE status='sending' AND json_extract(payload,'$.type')='ocApi' AND json_extract(payload,'$.request.type') IN ('context','member','chats')", [])?;
        // 取り出しただけの操作は再待機、通信開始済みは勝手に再投稿しない。
        db.execute(
            "UPDATE actions SET status='queued' WHERE status IN ('claimed','preparing','querying')",
            [],
        )?;
        db.execute("UPDATE actions SET status='unknown', code='RestartDuringSend', completed=?1 WHERE status='sending'", [now_ms()])?;
        // 廃止した試験の未送信分を取消す。unknownは照合用に残す。
        db.execute("UPDATE actions SET status='failed',code='CommandRetired',completed=?1,payload='' WHERE status IN ('queued','claimed','querying')
            AND json_extract(payload,'$.type')='ocApi' AND json_extract(json_extract(payload,'$.continuation'),'$.input.name')='test'
            AND json_extract(json_extract(payload,'$.continuation'),'$.input.args[0]')='mention-label'", [now_ms()])?;
        // Commandは受付でActionへ変換済み。本文は長期ログではない。
        db.execute("UPDATE events SET payload='' WHERE payload<>''", [])?;
        db.execute(
            "UPDATE actions SET payload='' WHERE status IN ('sent','failed')",
            [],
        )?;
        db.execute("DELETE FROM sessions WHERE expires<=?1", [now_ms()])?;
        Ok(Self {
            database: Mutex::new(db),
            wake: Notify::new(),
            stopped: AtomicBool::new(false),
            next_cleanup_ms: AtomicI64::new(0),
            max_retained_events,
            permissions,
            content,
            search_path,
            search_live: config.search_data_live.unwrap_or(false),
            assets: assets::AssetService::new("main")?,
            media_root,
            ffmpeg_path: config.ffmpeg_path.map(Into::into),
            cancel: tokio_util::sync::CancellationToken::new(),
            media_worker_active: AtomicBool::new(false),
            logs_enabled: config.logs_enabled.unwrap_or(false),
            store_worker_active: AtomicBool::new(false),
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

    pub fn persistence_revision(&self) -> Result<String> {
        Ok(self
            .database
            .lock()
            .map_err(|_| "DatabaseLock")?
            .total_changes()
            .to_string())
    }

    pub fn snapshot_database(&self, path: &str) -> Result<()> {
        self.database
            .lock()
            .map_err(|_| "DatabaseLock")?
            .execute("VACUUM INTO ?1", [path])?;
        Ok(())
    }

    pub fn pending_logs(&self) -> Result<Vec<PendingLog>> {
        logs::pending(&*self.database.lock().map_err(|_| "DatabaseLock")?)
    }

    pub fn priority_chats(&self) -> Result<Vec<String>> {
        let db = self.database.lock().map_err(|_| "DatabaseLock")?;
        oc::priority_chats(&db)
    }

    pub fn acknowledge_logs(&self, sequences: Vec<u32>) -> Result<()> {
        logs::acknowledge(
            &mut *self.database.lock().map_err(|_| "DatabaseLock")?,
            sequences,
        )
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
        // 検索時だけ公開データの索引を読み、SQLiteのlockを持って走査しない。
        for event in &batch.events {
            bytes += serde_json::to_string(event)?.len();
            if bytes > 256 * 1024 {
                return Err("BatchByteLimit".into());
            }
            if let CoreEvent::ReactionNotified {
                event_id,
                chat_id,
                message_id,
                reaction_type,
                created_at_ms,
            } = event
                && ([event_id, chat_id, message_id]
                    .iter()
                    .any(|id| id.is_empty() || id.len() > 256)
                    || !matches!(reaction_type.as_str(), "NICE" | "LOVE")
                    || *created_at_ms < 0)
            {
                return Err("InvalidReactionEvent".into());
            }
            let CoreEvent::MessageReceived {
                event_id,
                chat_id,
                message_id,
                text,
                sender_id,
                reply_to_message_id,
                created_at_ms,
                ..
            } = event
            else {
                if let CoreEvent::MemberChanged {
                    event_id,
                    square_id,
                    chat_id,
                    member_id,
                    display_name,
                    scope,
                    state,
                    created_at_ms,
                    ..
                } = event
                    && ([event_id, square_id, chat_id, member_id]
                        .iter()
                        .any(|id| id.is_empty() || id.len() > 256)
                        || display_name.len() > 320
                        || scope.len() > 16
                        || state.len() > 16
                        || *created_at_ms < 0)
                {
                    return Err("InvalidMemberEvent".into());
                }
                plans.push(commands::CommandPlan::Ignore);
                continue;
            };
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
                    commands::prepare(
                        &self.content,
                        &self.search_path,
                        self.search_live,
                        text,
                        now,
                    )
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
                (SELECT 1 FROM actions WHERE event_id=events.id AND status IN ('queued','preparing','claimed','querying','sending','unknown'))", [now - RETENTION_MS])?;
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
        let first_row: i64 =
            tx.query_row("SELECT COALESCE(max(rowid),0) FROM actions", [], |r| {
                r.get(0)
            })?;
        for (event, plan) in batch.events.into_iter().zip(plans) {
            let (event_id, _, _, _, created_at_ms) = oc::identity(&event);
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
                .is_some_and(|before| created_at_ms < before)
            {
                continue;
            }
            if self.logs_enabled {
                logs::ingest(&tx, &event)?;
            }
            if !oc::ingest(self, &tx, &event, &plan, now)? {
                self.apply_command(&tx, &event, plan, now)?;
            }
        }
        receipt.actions_created = tx.query_row(
            "SELECT count(*) FROM actions WHERE rowid>?1",
            [first_row],
            |r| r.get(0),
        )?;
        if self.logs_enabled {
            logs::check_capacity(&tx)?;
        }
        let event_count: i64 = tx.query_row("SELECT count(*) FROM events", [], |row| row.get(0))?;
        let action_count: i64 = tx.query_row(
            "SELECT count(*) FROM actions WHERE status IN ('queued','preparing','claimed','querying','sending','unknown')",
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

    fn apply_command(
        &self,
        tx: &rusqlite::Transaction<'_>,
        event: &CoreEvent,
        plan: commands::CommandPlan,
        now: i64,
    ) -> Result<()> {
        let (event_id, chat_id, _, _, _) = oc::identity(event);
        // 通常会話・OC管理・pingでは索引を読まない。検索候補はこの処理中だけ保持する。
        let needs_search = matches!(plan, commands::CommandPlan::Search(_))
            || (matches!(plan, commands::CommandPlan::Ignore)
                && if let CoreEvent::MessageReceived {
                    chat_id,
                    sender_id,
                    reply_to_message_id,
                    ..
                } = event
                {
                    reply_to_message_id.is_some() && tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sessions WHERE chat=?1 AND owner=?2 AND prompt=?3 AND expires>?4)",
                        params![chat_id, sender_id, reply_to_message_id, now], |row| row.get::<_, bool>(0))?
                } else {
                    false
                });
        let search = if needs_search {
            match commands::search::SearchCatalog::load(
                &self.search_path,
                std::sync::Arc::clone(&self.content.messages),
                self.search_live,
            ) {
                Ok(search) => Some(search),
                Err(error) => {
                    eprintln!("Search data unavailable: {error}");
                    None
                }
            }
        } else {
            None
        };
        let (responses, replacement) = commands::sessions::apply(
            tx,
            event,
            plan,
            search.as_ref(),
            &self.content.messages,
            now,
        )?;
        self.enqueue_responses(tx, event_id, chat_id, responses, replacement, now)
    }

    fn enqueue_responses(
        &self,
        tx: &rusqlite::Transaction<'_>,
        event_id: &str,
        chat_id: &str,
        mut responses: Vec<commands::sessions::Response>,
        replacement: Option<String>,
        now: i64,
    ) -> Result<()> {
        let message_catalog = &self.content.messages;
        if responses.iter().any(|(_, _, media)| media.is_some()) {
            let count: i64 = tx.query_row("SELECT count(*) FROM actions WHERE status IN ('queued','preparing','claimed','querying','sending','unknown') AND (json_extract(payload,'$.type')='prepareMedia' OR json_extract(payload,'$.attachment') IS NOT NULL)", [], |row|row.get(0))?;
            if count >= media::MAX_MEDIA_JOBS {
                responses = vec![(
                    message!(message_catalog, "media.enqueue_responses_01").into(),
                    now,
                    None,
                )];
                tx.execute("DELETE FROM sessions WHERE id=?1", [event_id])?;
            }
        }
        let responses = commands::split_responses(responses)?;
        for (index, (body, due, media)) in responses.into_iter().enumerate() {
            let id = format!("{event_id}:{index}");
            let is_prompt: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE action=?1)",
                [&id],
                |row| row.get(0),
            )?;
            let replace_message_id = if index == 0 {
                replacement.clone()
            } else {
                None
            };
            let action = if let Some(request) = media {
                CoreAction::PrepareMedia {
                    action_id: id.clone(),
                    event_id: event_id.into(),
                    chat_id: chat_id.into(),
                    related_message_id: String::new(),
                    request: serde_json::to_string(&request)?,
                    replace_message_id,
                    is_prompt,
                    created_at_ms: now,
                }
            } else {
                CoreAction::SendMessage {
                    action_id: id.clone(),
                    event_id: event_id.into(),
                    chat_id: chat_id.into(),
                    related_message_id: String::new(),
                    emojis: None,
                    text: body,
                    image_url: None,
                    attachment: None,
                    mention: None,
                    replace_message_id,
                    is_prompt,
                    created_at_ms: now,
                }
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
        }
        Ok(())
    }

    pub async fn next_action(&self) -> Result<Option<CoreAction>> {
        self.next_action_mode(false).await
    }

    pub async fn next_query_action(&self) -> Result<Option<CoreAction>> {
        self.next_action_mode(true).await
    }

    async fn next_action_mode(&self, query: bool) -> Result<Option<CoreAction>> {
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
                    AND json_extract(a.payload,'$.type')<>'prepareMedia'
                    AND ?1=COALESCE(json_extract(a.payload,'$.type')='ocApi'
                        AND json_extract(a.payload,'$.request.type') IN ('context','member','chats','members','joinedChats','inspect','reactions'),0)
                    AND (?1=1 OR NOT EXISTS (SELECT 1 FROM actions b WHERE b.chat=a.chat AND b.status IN ('claimed','sending')))
                    ORDER BY a.due,a.rowid LIMIT 1", [query],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
                match next {
                    Some((id, payload, due)) if due <= now_ms() => {
                        let action = serde_json::from_str(&payload)?;
                        db.execute(
                            "UPDATE actions SET status=?2 WHERE id=?1",
                            params![id, if query { "querying" } else { "claimed" }],
                        )?;
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
            "UPDATE actions SET status=CASE WHEN status='querying' THEN 'querying' ELSE 'sending' END WHERE id=?1 AND status IN ('claimed','querying')",
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
        let previous = match result.status {
            DeliveryStatus::Sent => "completed",
            DeliveryStatus::Failed => "failed_result",
            DeliveryStatus::Unknown => "sending",
        };
        self.finish_action(result, previous)
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
                "SELECT payload FROM actions WHERE id=?1 AND (status=?2
                 OR (?2='completed' AND status IN ('sending','querying'))
                 OR (?2='failed_result' AND status IN ('claimed','sending','querying')))",
                params![result.action_id, previous],
                |row| row.get(0),
            )
            .optional()?
            .ok_or("InvalidActionState")?;
        let completed: CoreAction = serde_json::from_str(&payload)?;
        let row_id: i64 = tx.query_row(
            "SELECT rowid FROM actions WHERE id=?1",
            [&result.action_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE actions SET status=?2,code=?3,completed=?4,
            payload=CASE WHEN ?2 IN ('sent','failed') THEN '' ELSE payload END WHERE id=?1",
            params![result.action_id, status, result.code, now_ms()],
        )?;
        if matches!(result.status, DeliveryStatus::Sent) {
            if matches!(completed, CoreAction::SendMessage { .. }) {
                let waiting: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE action=?1 AND expires>?2)",
                    params![result.action_id, now_ms()],
                    |row| row.get(0),
                )?;
                if waiting && result.message_id.is_none() {
                    return Err("MissingPromptMessageId".into());
                }
                tx.execute(
                "UPDATE sessions SET prompt=?2,expires=?3,payload=COALESCE(pending_payload,payload),pending_payload=NULL WHERE action=?1 AND expires>?4",
                params![
                    result.action_id,
                    result.message_id,
                    now_ms() + commands::search::SESSION_TTL_MS,
                    now_ms()
                ],
            )?;
            }
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
            tx.execute("DELETE FROM sessions WHERE action=?1 AND (prompt IS NULL OR pending_payload IS NULL)", [&result.action_id])?;
            tx.execute(
                "UPDATE sessions SET pending_payload=NULL WHERE action=?1",
                [&result.action_id],
            )?;
        }
        oc::complete(
            self,
            &tx,
            &completed,
            &result,
            previous == "unknown",
            now_ms(),
        )?;
        trim_completed(&tx)?;
        tx.commit()?;
        if matches!(result.status, DeliveryStatus::Sent | DeliveryStatus::Failed) {
            let directory = self.media_root.join(format!("job-{row_id}"));
            if directory.is_dir() {
                let _ = std::fs::remove_dir_all(directory);
            }
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
            max_retained_events: self.max_retained_events,
            queued_actions: count("queued")?,
            claimed_actions: count("claimed")?,
            querying_actions: count("querying")?,
            sending_actions: count("sending")?,
            unknown_actions: count("unknown")?,
            failed_actions: count("failed")?,
            completed_actions: count("sent")? + count("failed")?,
            active_sessions: db.query_row(
                "SELECT count(*) FROM sessions WHERE expires>?1",
                [now_ms()],
                |row| row.get(0),
            )?,
            preparing_media: count("preparing")?,
            pending_logs: db.query_row("SELECT count(*) FROM log_pending", [], |r| r.get(0))?,
            pending_log_bytes: db.query_row(
                "SELECT COALESCE(sum(bytes),0) FROM log_pending",
                [],
                |r| r.get(0),
            )?,
        })
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.cancel.cancel();
        self.wake.notify_waiters();
    }

    pub async fn prepare_image(&self, action_id: &str) -> Result<Option<Vec<u8>>> {
        let message_catalog = &self.content.messages;
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
        let download = async {
            let search = commands::search::SearchCatalog::load(
                &self.search_path,
                std::sync::Arc::clone(message_catalog),
                self.search_live,
            )?;
            let assets = self.assets.at_revision(search.asset_commit()?)?;
            drop(search);
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                assets.download(reqwest::Url::parse(url)?).await?,
            )
        };
        match download.await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => {
                // 取得失敗はLINEへ何も送っていない。元URLを示す通常返信へ切り替える。
                *text = message!(message_catalog, "media.prepare_image_01", url = url);
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

fn trim_completed(db: &Connection) -> Result<()> {
    db.execute("DELETE FROM actions WHERE id IN (SELECT id FROM actions WHERE status IN ('sent','failed') ORDER BY completed DESC,rowid DESC LIMIT -1 OFFSET ?1)", [MAX_COMPLETED_ACTIONS])?;
    Ok(())
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
            "SELECT count(*) FROM actions WHERE status IN ('queued','preparing','claimed','querying','sending','unknown')",
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
