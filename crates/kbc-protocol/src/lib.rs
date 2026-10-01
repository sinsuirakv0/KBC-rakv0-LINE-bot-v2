use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreConfig {
    pub database_path: String,
    pub owner_id: String,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoreEvent {
    MessageReceived {
        event_id: String,
        chat_id: String,
        message_id: String,
        text: String,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedBatch {
    pub protocol_version: u32,
    pub stream_key: String,
    pub checkpoint: String,
    #[ts(type = "number | null")]
    pub baseline_before_ms: Option<i64>,
    pub events: Vec<CoreEvent>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoreAction {
    SendMessage {
        action_id: String,
        event_id: String,
        chat_id: String,
        related_message_id: String,
        text: String,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryStatus {
    Sent,
    Failed,
    Unknown,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub action_id: String,
    pub status: DeliveryStatus,
    pub code: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BatchReceipt {
    pub accepted: u32,
    pub duplicates: u32,
    pub actions_created: u32,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreStats {
    pub retained_events: u32,
    pub queued_actions: u32,
    pub sending_actions: u32,
    pub unknown_actions: u32,
    pub failed_actions: u32,
}
