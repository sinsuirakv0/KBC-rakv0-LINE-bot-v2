use crate::messages::message;
use crate::{
    Result, Runtime,
    commands::search::SearchSession,
    motion::{MotionJob, MotionPlan},
    now_ms,
};
use kbc_protocol::{Attachment, CoreAction};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const MAX_MEDIA_JOBS: i64 = 8;
pub const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub enum MediaRequest {
    Schedule { date: Option<String> },
    Download { path: String },
    FileList { session_id: String },
    Motion(Box<MotionPlan>),
}

#[derive(Serialize, Deserialize)]
pub struct MediaJob {
    pub catalog_revision: String,
    pub request: MediaRequest,
}

pub struct Artifact {
    pub path: PathBuf,
    pub file_name: String,
    pub content_type: Option<String>,
    pub duration_ms: Option<u32>,
}
impl Artifact {
    pub fn new(
        path: PathBuf,
        file_name: impl Into<String>,
        content_type: Option<String>,
        duration_ms: Option<u32>,
    ) -> Self {
        Self {
            path,
            file_name: file_name.into(),
            content_type,
            duration_ms,
        }
    }
}
#[derive(Clone)]
pub struct RenderContext {
    workspace: Arc<PathBuf>,
    cancellation: CancellationToken,
    queue_wait: Duration,
    memory_peak: Arc<std::sync::atomic::AtomicU64>,
}
impl RenderContext {
    pub fn output_path(&self, name: &str) -> Result<PathBuf> {
        let path = Path::new(name);
        if path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
        {
            return Err("InvalidOutputName".into());
        }
        Ok(self.workspace.join(path))
    }
    // LINEでは受付と完成だけを投稿し、描画進捗ごとのAPI通信を作らない。
    pub fn report(&self, _content: impl Into<String>) {}
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
    pub fn queue_wait(&self) -> Duration {
        self.queue_wait
    }
    pub fn check_memory(&self, additional_bytes: u64) -> Result<()> {
        // 子プロセスも含むコンテナ使用量で判定し、受信・保存用に64MiBを残す。
        let paths = [
            ("/sys/fs/cgroup/memory.current", "/sys/fs/cgroup/memory.max"),
            (
                "/sys/fs/cgroup/memory/memory.usage_in_bytes",
                "/sys/fs/cgroup/memory/memory.limit_in_bytes",
            ),
        ];
        for (current_path, limit_path) in paths {
            let current = std::fs::read_to_string(current_path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok());
            let limit = std::fs::read_to_string(limit_path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok());
            if let (Some(current), Some(limit)) = (current, limit) {
                self.memory_peak
                    .fetch_max(current, std::sync::atomic::Ordering::Relaxed);
                if current.saturating_add(additional_bytes) > limit.saturating_sub(64 * 1024 * 1024)
                {
                    return Err("MediaMemoryBudgetExceeded".into());
                }
                break;
            }
        }
        Ok(())
    }
    pub fn memory_peak_bytes(&self) -> u64 {
        self.memory_peak.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Runtime {
    pub async fn run_media_jobs(&self) -> Result<()> {
        if self
            .media_worker_active
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Err("MediaWorkerAlreadyRunning".into());
        }
        // 呼出は一つだけ。生成を通常の配送枠・トーク直列制御から分ける。
        let result = self.media_loop().await;
        self.media_worker_active
            .store(false, std::sync::atomic::Ordering::Release);
        result
    }
    async fn media_loop(&self) -> Result<()> {
        self.prune_media().await?;
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            let next: Option<(i64, String, String, String)> = {
                let db = self.database.lock().map_err(|_| "DatabaseLock")?;
                let row = db.query_row("SELECT rowid,id,payload,code FROM actions WHERE status='queued' AND json_extract(payload,'$.type')='prepareMedia' ORDER BY rowid LIMIT 1", [], |row| Ok((row.get(0)?, row.get::<_,String>(1)?, row.get(2)?, row.get(3)?))).optional()?;
                if let Some((_, id, _, _)) = &row {
                    db.execute("UPDATE actions SET status='preparing' WHERE id=?1", [id])?;
                }
                row
            };
            let Some((row_id, action_id, payload, previous_code)) = next else {
                notified.await;
                continue;
            };
            let action: CoreAction = serde_json::from_str(&payload)?;
            let CoreAction::PrepareMedia {
                request,
                created_at_ms,
                ..
            } = &action
            else {
                return Err("InvalidMediaJob".into());
            };
            let job: MediaJob = serde_json::from_str(request)?;
            let is_schedule = matches!(&job.request, MediaRequest::Schedule { .. });
            let is_motion = matches!(&job.request, MediaRequest::Motion(_));
            let resume_remote = is_motion
                && self.motion_remote.is_some()
                && matches!(
                    previous_code.as_str(),
                    "MotionLocalRunning" | "MotionRemotePending"
                );
            let execution_seconds = if is_motion && self.motion_remote.is_some() {
                1860
            } else {
                600
            };
            let workspace = self.media_root.join(format!("job-{row_id}"));
            tokio::fs::create_dir_all(&workspace).await?;
            let context = RenderContext {
                workspace: Arc::new(workspace.clone()),
                cancellation: self.cancel.child_token(),
                queue_wait: Duration::from_millis((now_ms() - created_at_ms).max(0) as u64),
                memory_peak: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            };
            // 再開時は初回の待機期限で生成済み状態を捨てず、依頼全体の期限を延長しない。
            let execution_limit = Duration::from_secs(execution_seconds).min(
                Duration::from_secs(600 + execution_seconds).saturating_sub(context.queue_wait),
            );
            let future = async {
                if let MediaRequest::Schedule { date } = &job.request {
                    return self
                        .prepare_schedule(date.clone())
                        .await
                        .map(MediaOutput::Thread);
                }
                let search = crate::commands::search::SearchCatalog::load(
                    &self.search_path,
                    Arc::clone(&self.content.messages),
                    self.search_live,
                )?;
                if job.catalog_revision != search.revision {
                    return Err("SearchDataChanged".into());
                }
                let assets = self.assets.at_revision(search.asset_commit()?)?;
                drop(search);
                if !is_motion {
                    context.check_memory(0)?;
                }
                self.generate_media(
                    &action_id,
                    job.request,
                    assets,
                    context.clone(),
                    resume_remote,
                )
                .await
            };
            tokio::pin!(future);
            let outcome = if (!resume_remote && context.queue_wait > Duration::from_secs(600))
                || execution_limit.is_zero()
            {
                Err("MediaQueueExpired".into())
            } else {
                tokio::select! {
                    biased;
                    _ = self.cancel.cancelled() => {
                        context.cancellation.cancel();
                        if tokio::time::timeout(Duration::from_secs(15), &mut future).await.is_err() { return Err("MediaCancellationStalled".into()); }
                        self.database.lock().map_err(|_| "DatabaseLock")?.execute("UPDATE actions SET status='queued' WHERE id=?1 AND status='preparing'", [&action_id])?;
                        return Ok(());
                    },
                    result = &mut future => result,
                    _ = tokio::time::sleep(execution_limit) => {
                        context.cancellation.cancel();
                        if tokio::time::timeout(Duration::from_secs(15), &mut future).await.is_err() { return Err("MediaCancellationStalled".into()); }
                        Err("MediaExecutionTimeout".into())
                    },
                }
            };
            if outcome
                .as_ref()
                .err()
                .is_some_and(|error| error.to_string() == "MediaCancellationStalled")
            {
                return Err("MediaCancellationStalled".into());
            }
            self.finish_media(action, outcome, is_schedule)?;
            let retained: bool = self.database.lock().map_err(|_|"DatabaseLock")?.query_row("SELECT EXISTS(SELECT 1 FROM actions WHERE id=?1 AND json_extract(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,'$.attachment') IS NOT NULL)",[&action_id],|row|row.get(0))?;
            if !retained {
                tokio::fs::remove_dir_all(&workspace).await?;
            }
            self.wake.notify_waiters();
        }
    }
    async fn prune_media(&self) -> Result<()> {
        let retained: std::collections::HashSet<i64> = {
            let db = self.database.lock().map_err(|_| "DatabaseLock")?;
            let mut statement=db.prepare("SELECT rowid FROM actions WHERE status IN ('queued','preparing','claimed','querying','sending','unknown') AND (json_extract(payload,'$.type')='prepareMedia' OR json_extract(payload,'$.attachment') IS NOT NULL)")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut entries = tokio::fs::read_dir(&self.media_root).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir()
                && let Some(id) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_prefix("job-"))
                    .and_then(|id| id.parse::<i64>().ok())
                && !retained.contains(&id)
            {
                tokio::fs::remove_dir_all(entry.path()).await?;
            }
        }
        Ok(())
    }
    async fn generate_media(
        &self,
        action_id: &str,
        request: MediaRequest,
        assets: crate::assets::AssetService,
        context: RenderContext,
        resume_remote: bool,
    ) -> Result<MediaOutput> {
        match request {
            MediaRequest::Schedule { .. } => Err("InvalidMediaRequest".into()),
            MediaRequest::FileList { session_id } => {
                let stored: Option<String> = self
                    .database
                    .lock()
                    .map_err(|_| "DatabaseLock")?
                    .query_row(
                        "SELECT payload FROM sessions WHERE id=?1 AND action=?2 AND expires>?3",
                        params![session_id, action_id, now_ms()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(payload) = stored else {
                    return Ok(MediaOutput::Superseded);
                };
                let mut session: SearchSession = serde_json::from_str(&payload)?;
                let mut files = Vec::new();
                for pair in session.files.take().ok_or("MissingFileOptions")?.chunks(2) {
                    let first = assets.exists(&pair[0].path);
                    if pair.len() == 2 {
                        let (a, b) = tokio::try_join!(first, assets.exists(&pair[1].path))?;
                        if a {
                            files.push(pair[0].clone());
                        }
                        if b {
                            files.push(pair[1].clone());
                        }
                    } else if first.await? {
                        files.push(pair[0].clone());
                    }
                }
                if files.is_empty() {
                    return Err("NoAvailableFiles".into());
                }
                session.files = Some(files);
                // 索引全体を非同期処理の間保持せず、一覧文面だけを成果へ残す。
                let search = crate::commands::search::SearchCatalog::load(
                    &self.search_path,
                    Arc::clone(&self.content.messages),
                    self.search_live,
                )?;
                if session.revision != search.session_revision() {
                    return Err("SearchDataChanged".into());
                }
                let text = search.page(&session, true);
                Ok(MediaOutput::FileList {
                    session_id,
                    session,
                    text,
                })
            }
            MediaRequest::Download { path } => {
                let bytes = assets.bytes(&path).await?;
                if path.ends_with(".png") && !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return Err("InvalidImageType".into());
                }
                let output = context.output_path("output")?;
                tokio::fs::write(&output, bytes).await?;
                Ok(MediaOutput::Attachment {
                    artifact: Artifact::new(
                        output,
                        path.rsplit('/').next().ok_or("InvalidAssetPath")?,
                        Some(
                            if path.ends_with(".png") {
                                "image/png"
                            } else {
                                "application/octet-stream"
                            }
                            .into(),
                        ),
                        None,
                    ),
                    duration_ms: None,
                })
            }
            MediaRequest::Motion(plan) => {
                let artifact = self
                    .render_motion(action_id, *plan, assets, context, resume_remote)
                    .await?;
                let duration_ms = artifact.duration_ms;
                Ok(MediaOutput::Attachment {
                    artifact,
                    duration_ms,
                })
            }
        }
    }

    async fn render_motion(
        &self,
        action_id: &str,
        plan: MotionPlan,
        assets: crate::assets::AssetService,
        context: RenderContext,
        resume_remote: bool,
    ) -> Result<Artifact> {
        use crate::motion::{MotionError, request::MotionFormat};
        if !resume_remote || self.motion_remote.is_none() {
            if self.motion_remote.is_some() {
                self.database.lock().map_err(|_| "DatabaseLock")?.execute(
                    "UPDATE actions SET code='MotionLocalRunning' WHERE id=?1 AND status='preparing'", [action_id])?;
            }
            let local_context = RenderContext {
                cancellation: context.cancellation.child_token(),
                ..context.clone()
            };
            let local = async {
                local_context
                    .check_memory(0)
                    .map_err(|error| MotionError::render(error.to_string()))?;
                if plan.format != MotionFormat::Png
                    && self.ffmpeg_path.as_ref().is_none_or(|path| !path.is_file())
                {
                    return Err(MotionError::render("MissingFfmpeg"));
                }
                MotionJob::new(plan.clone(), assets.clone(), self.ffmpeg_path.clone())
                    .render(local_context.clone())
                    .await
            };
            tokio::pin!(local);
            let outcome = match tokio::time::timeout(Duration::from_secs(600), &mut local).await {
                Ok(outcome) => outcome,
                Err(_) => {
                    local_context.cancellation.cancel();
                    if tokio::time::timeout(Duration::from_secs(15), &mut local)
                        .await
                        .is_err()
                    {
                        return Err("MediaCancellationStalled".into());
                    }
                    Err(MotionError::render("MediaExecutionTimeout"))
                }
            };
            match outcome {
                Ok(artifact) => return Ok(artifact),
                Err(error) => {
                    eprintln!("Motion local generation failed: {error}");
                    if !error.can_delegate()
                        || self.motion_remote.is_none()
                        || context.cancellation.is_cancelled()
                    {
                        return Err(match error.to_string().as_str() {
                            "MissingFfmpeg" | "MediaMemoryBudgetExceeded" => error.to_string(),
                            _ => error.public_message().into(),
                        }
                        .into());
                    }
                }
            }
        }
        let remote = self
            .motion_remote
            .as_ref()
            .ok_or("MotionRemoteUnavailable")?;
        // 再起動時にローカル重処理を繰り返さず、同じ代行依頼を再開する。
        self.database.lock().map_err(|_| "DatabaseLock")?.execute(
            "UPDATE actions SET code='MotionRemotePending' WHERE id=?1 AND status='preparing'",
            [action_id],
        )?;
        use sha1::{Digest, Sha1};
        let id = format!("{:x}", Sha1::digest(action_id.as_bytes()));
        eprintln!("Motion generation delegated: request_id={id}");
        remote.render(&id, &plan, &assets, &context).await
    }
    fn finish_media(
        &self,
        action: CoreAction,
        outcome: Result<MediaOutput>,
        is_schedule: bool,
    ) -> Result<()> {
        let message_catalog = &self.content.messages;
        let CoreAction::PrepareMedia {
            action_id,
            event_id,
            chat_id,
            related_message_id,
            replace_message_id,
            mut is_prompt,
            created_at_ms,
            ..
        } = action
        else {
            return Err("InvalidMediaJob".into());
        };
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        let mut text = String::new();
        let mut attachment = None;
        let mut file_list_ready = false;
        let mut thread_contents = None;
        match outcome {
            Ok(MediaOutput::Thread(contents)) => {
                text = message!(message_catalog, "skd.root").into();
                thread_contents = Some(contents);
            }
            Ok(MediaOutput::Superseded) => {
                tx.execute("UPDATE actions SET status='failed',payload='',code='Superseded',completed=?2 WHERE id=?1 AND status='preparing'",params![action_id,now_ms()])?;
                crate::trim_completed(&tx)?;
                tx.commit()?;
                return Ok(());
            }
            Ok(MediaOutput::FileList {
                session_id,
                session,
                text: page,
            }) => {
                let changed=tx.execute("UPDATE sessions SET payload=?3,expires=?4 WHERE id=?1 AND action=?2 AND expires>?5",params![session_id,action_id,serde_json::to_string(&session)?,now_ms()+crate::commands::search::SESSION_TTL_MS,now_ms()])?;
                if changed == 0 {
                    tx.execute("UPDATE actions SET status='failed',payload='',code='Superseded',completed=?2 WHERE id=?1",params![action_id,now_ms()])?;
                    crate::trim_completed(&tx)?;
                    tx.commit()?;
                    return Ok(());
                }
                text = page;
                file_list_ready = true;
            }
            Ok(MediaOutput::Attachment {
                artifact,
                duration_ms,
            }) => {
                let row_id: i64 =
                    tx.query_row("SELECT rowid FROM actions WHERE id=?1", [&action_id], |r| {
                        r.get(0)
                    })?;
                if std::fs::metadata(&artifact.path)?.len() > MAX_OUTPUT_BYTES {
                    text = message!(message_catalog, "media.finish_media_01").into();
                } else {
                    let output = self.media_root.join(format!("job-{row_id}")).join("output");
                    if artifact.path != output {
                        std::fs::rename(&artifact.path, &output)?;
                    }
                    let content_type = artifact
                        .content_type
                        .unwrap_or("application/octet-stream".into());
                    let kind = match content_type.as_str() {
                        "image/png" => "image",
                        "image/gif" => "gif",
                        "video/mp4" => "video",
                        _ => "file",
                    };
                    attachment = Some(Attachment {
                        file_name: artifact.file_name,
                        content_type,
                        kind: kind.into(),
                        duration_ms,
                    });
                }
            }
            Err(error) => {
                eprintln!("Media preparation failed: {error}");
                text = if is_schedule {
                    message!(message_catalog, "skd.failed")
                } else if error.to_string() == "MediaMemoryBudgetExceeded" {
                    message!(message_catalog, "media.finish_media_02")
                } else if error.to_string() == "MissingFfmpeg" {
                    message!(message_catalog, "media.finish_media_03")
                } else if error.to_string() == "NoAvailableFiles" {
                    message!(message_catalog, "media.finish_media_04")
                } else if error.to_string() == "SearchDataChanged" {
                    message!(message_catalog, "media.finish_media_05")
                } else {
                    message!(message_catalog, "media.finish_media_06")
                }
                .into();
            }
        }
        if is_prompt && !file_list_ready {
            tx.execute("DELETE FROM sessions WHERE action=?1", [&action_id])?;
            is_prompt = false;
        }
        let prepared = CoreAction::SendMessage {
            action_id: action_id.clone(),
            event_id,
            chat_id,
            related_message_id,
            emojis: None,
            text,
            thread_root_id: None,
            thread_contents,
            image_url: None,
            attachment,
            mention: None,
            replace_message_id,
            is_prompt,
            created_at_ms,
        };
        tx.execute("UPDATE actions SET status='queued',payload=?2,due=?3,code='' WHERE id=?1 AND status='preparing'",params![action_id,serde_json::to_string(&prepared)?,now_ms()])?;
        tx.commit()?;
        Ok(())
    }
    pub async fn prepare_attachment(&self, action_id: &str) -> Result<Option<Vec<u8>>> {
        let message_catalog = &self.content.messages;
        let (row_id,payload):(i64,String)=self.database.lock().map_err(|_|"DatabaseLock")?.query_row("SELECT rowid,payload FROM actions WHERE id=?1 AND status='claimed' AND json_extract(payload,'$.attachment') IS NOT NULL",[action_id],|row|Ok((row.get(0)?,row.get(1)?)))?;
        let path = self.media_root.join(format!("job-{row_id}")).join("output");
        let read = async {
            if tokio::fs::metadata(&path).await?.len() > MAX_OUTPUT_BYTES {
                return Err("MediaByteLimit".into());
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(tokio::fs::read(path).await?)
        }
        .await;
        match read {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => {
                // 成果消失は未通信。1件の再準備案内に変え、通常の配送を止めない。
                let mut action: CoreAction = serde_json::from_str(&payload)?;
                let CoreAction::SendMessage {
                    attachment, text, ..
                } = &mut action
                else {
                    return Err("InvalidAttachmentAction".into());
                };
                *attachment = None;
                *text = message!(message_catalog, "media.prepare_attachment_01").into();
                let changed=self.database.lock().map_err(|_|"DatabaseLock")?.execute("UPDATE actions SET payload=?2,status='queued',code='MediaUnavailable' WHERE id=?1 AND status='claimed'",params![action_id,serde_json::to_string(&action)?])?;
                if changed != 1 {
                    return Err("ActionNotClaimed".into());
                }
                self.wake.notify_waiters();
                Ok(None)
            }
        }
    }
}
enum MediaOutput {
    Thread(Vec<String>),
    Superseded,
    FileList {
        session_id: String,
        session: SearchSession,
        text: String,
    },
    Attachment {
        artifact: Artifact,
        duration_ms: Option<u32>,
    },
}
