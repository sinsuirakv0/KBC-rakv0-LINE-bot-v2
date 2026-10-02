use kbc_core::Runtime;
use napi::{Error, Result};
use napi_derive::napi;
use serde_json::Value;

fn convert<T>(result: std::result::Result<T, impl std::fmt::Display>) -> Result<T> {
    result.map_err(|error| Error::from_reason(error.to_string()))
}

#[napi]
pub struct NativeCore {
    runtime: Runtime,
}

#[napi]
impl NativeCore {
    #[napi]
    pub fn checkpoint(&self, stream: String) -> Result<Option<String>> {
        convert(self.runtime.checkpoint(&stream))
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
    pub async fn next_action(&self) -> Result<Option<Value>> {
        let action = convert(self.runtime.next_action().await)?;
        action
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
}

#[napi]
pub fn create_core(config: String) -> Result<NativeCore> {
    Ok(NativeCore {
        runtime: convert(Runtime::open(convert(serde_json::from_str(&config))?))?,
    })
}

#[napi]
pub fn get_runtime_info() -> Value {
    serde_json::json!({ "protocolVersion": kbc_protocol::PROTOCOL_VERSION, "coreVersion": env!("CARGO_PKG_VERSION") })
}
