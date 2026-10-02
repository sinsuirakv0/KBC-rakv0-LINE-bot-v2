use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 3;

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreConfig {
    pub database_path: String,
    pub owner_id: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_retained_events: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub search_data_path: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        sender_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        reply_to_message_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        image_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        replace_message_id: Option<String>,
        #[serde(default)]
        is_prompt: bool,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
    DeleteMessage {
        action_id: String,
        event_id: String,
        chat_id: String,
        message_id: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message_id: Option<String>,
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
    pub max_retained_events: u32,
    pub queued_actions: u32,
    pub claimed_actions: u32,
    pub sending_actions: u32,
    pub unknown_actions: u32,
    pub failed_actions: u32,
    pub completed_actions: u32,
    pub active_sessions: u32,
}
