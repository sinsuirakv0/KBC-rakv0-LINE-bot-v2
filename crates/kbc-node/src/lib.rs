use kbc_core::Runtime;
use napi::bindgen_prelude::Buffer;
use napi::{Error, Result};
use napi_derive::napi;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

fn convert<T>(result: std::result::Result<T, impl std::fmt::Display>) -> Result<T> {
    result.map_err(|error| Error::from_reason(error.to_string()))
}

#[napi]
pub struct NativeCore {
    runtime: Arc<Runtime>,
    admission: Arc<Semaphore>,
    worker: Arc<Semaphore>,
}

#[napi]
impl NativeCore {
    #[napi]
    pub fn checkpoint(&self, stream: String) -> Result<Option<String>> {
        convert(self.runtime.checkpoint(&stream))
    }

    #[napi]
    pub fn priority_chats(&self) -> Result<Vec<String>> {
        convert(self.runtime.priority_chats())
    }

    #[napi]
    pub fn submit_batch(&self, batch: String) -> Result<Value> {
        let receipt = convert(
            self.runtime
                .submit_batch(convert(serde_json::from_str(&batch))?),
        )?;
        convert(serde_json::to_value(receipt))
    }

    #[napi]
    pub async fn submit_batch_async(&self, batch: String) -> Result<Value> {
        // 最大4受付・実処理1件。検索・fsyncでNodeのPUSH処理を塞がない。
        let admission = convert(Arc::clone(&self.admission).try_acquire_owned())?;
        let worker = convert(Arc::clone(&self.worker).acquire_owned().await)?;
        let runtime = Arc::clone(&self.runtime);
        let receipt = convert(convert(
            tokio::task::spawn_blocking(move || {
                let _permits = (admission, worker);
                runtime.submit_batch(serde_json::from_str(&batch)?)
            })
            .await,
        )?)?;
        convert(serde_json::to_value(receipt))
    }

    #[napi]
    pub async fn prepare_image(&self, action_id: String) -> Result<Option<Buffer>> {
        Ok(convert(self.runtime.prepare_image(&action_id).await)?.map(Buffer::from))
    }

    #[napi]
    pub async fn run_media_jobs(&self) -> Result<()> {
        convert(self.runtime.run_media_jobs().await)
    }

    #[napi]
    pub async fn prepare_attachment(&self, action_id: String) -> Result<Option<Buffer>> {
        Ok(convert(self.runtime.prepare_attachment(&action_id).await)?.map(Buffer::from))
    }

    #[napi]
    pub async fn next_action(&self) -> Result<Option<Value>> {
        let action = convert(self.runtime.next_action().await)?;
        action
            .map(|value| convert(serde_json::to_value(value)))
            .transpose()
    }

    #[napi]
    pub async fn next_query_action(&self) -> Result<Option<Value>> {
        convert(self.runtime.next_query_action().await)?
            .map(|value| convert(serde_json::to_value(value)))
            .transpose()
    }

    #[napi]
    pub fn complete_action(&self, result: String) -> Result<()> {
        convert(
            self.runtime
                .complete_action(convert(serde_json::from_str(&result))?),
        )
    }

    #[napi]
    pub fn mark_sending(&self, action_id: String) -> Result<()> {
        convert(self.runtime.mark_sending(&action_id))
    }

    #[napi]
    pub fn retry_action(&self, action_id: String, delay_ms: u32) -> Result<()> {
        convert(self.runtime.retry_action(&action_id, delay_ms))
    }

    #[napi]
    pub fn resolve_action(&self, result: String) -> Result<()> {
        convert(
            self.runtime
                .resolve_action(convert(serde_json::from_str(&result))?),
        )
    }

    #[napi]
    pub fn stats(&self) -> Result<Value> {
        convert(serde_json::to_value(convert(self.runtime.stats())?))
    }

    #[napi]
    pub fn shutdown(&self) {
        self.runtime.shutdown();
    }

    #[napi]
    pub fn persistence_revision(&self) -> Result<String> {
        convert(self.runtime.persistence_revision())
    }

    #[napi]
    pub fn pending_logs(&self) -> Result<Value> {
        convert(serde_json::to_value(convert(self.runtime.pending_logs())?))
    }

    #[napi]
    pub fn acknowledge_logs(&self, sequences: String) -> Result<()> {
        convert(
            self.runtime
                .acknowledge_logs(convert(serde_json::from_str(&sequences))?),
        )
    }

    #[napi]
    pub async fn snapshot_database(&self, path: String) -> Result<()> {
        let runtime = self.runtime.clone();
        convert(convert(
            tokio::task::spawn_blocking(move || runtime.snapshot_database(&path)).await,
        )?)
    }
}

#[napi]
pub fn create_core(config: String) -> Result<NativeCore> {
    Ok(NativeCore {
        runtime: Arc::new(convert(Runtime::open(convert(serde_json::from_str(
            &config,
        ))?))?),
        admission: Arc::new(Semaphore::new(4)),
        worker: Arc::new(Semaphore::new(1)),
    })
}

#[napi]
pub fn get_runtime_info() -> Value {
    serde_json::json!({ "protocolVersion": kbc_protocol::PROTOCOL_VERSION, "coreVersion": env!("CARGO_PKG_VERSION") })
}
