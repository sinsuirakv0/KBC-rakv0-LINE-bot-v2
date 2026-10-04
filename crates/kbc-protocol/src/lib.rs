use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 14;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ffmpeg_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub permissions_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub legacy_oc_settings_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub restored_from_backup: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub logs_enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoreEvent {
    ReactionNotified {
        event_id: String,
        chat_id: String,
        message_id: String,
        reaction_type: String,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        square_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        bot_member_id: Option<String>,
        #[serde(default)]
        #[ts(optional)]
        mentions: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        content_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        sender_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        metadata_json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        media_group_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        media_group_sequence: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        media_group_total: Option<u32>,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
    MemberChanged {
        event_id: String,
        square_id: String,
        chat_id: String,
        member_id: String,
        display_name: String,
        scope: String,
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        member_created_at_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        metadata_json: Option<String>,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PendingLog {
    pub sequence: u32,
    pub stream: String,
    pub row: String,
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
        mention: Option<MessageMention>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        emojis: Option<Vec<MessageEmoji>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        image_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        attachment: Option<Attachment>,
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
    PrepareMedia {
        action_id: String,
        event_id: String,
        chat_id: String,
        related_message_id: String,
        request: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        replace_message_id: Option<String>,
        is_prompt: bool,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
    OcApi {
        action_id: String,
        event_id: String,
        chat_id: String,
        request: OcRequest,
        continuation: String,
        #[ts(type = "number")]
        created_at_ms: i64,
    },
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageEmoji {
    pub product_id: String,
    pub emoji_id: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub file_name: String,
    pub content_type: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub duration_ms: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageMention {
    pub member_id: String,
    pub start: u32,
    pub end: u32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub oc_result: Option<OcResult>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OcRequest {
    Reactions {
        message_id: String,
        member_id: String,
        reaction_type: String,
    },
    Context {
        member_id: String,
        authority: bool,
    },
    Member {
        member_id: String,
    },
    Inspect {
        chat_id: String,
        member_ids: Vec<String>,
    },
    Chats {
        square_id: String,
    },
    JoinedChats {
        continuation_token: Option<String>,
    },
    Members {
        square_id: String,
        query: String,
        state: String,
        continuation_token: Option<String>,
    },
    Membership {
        square_id: String,
        member_id: String,
        revision: String,
        state: String,
    },
    Profile {
        square_id: String,
        member_id: String,
        revision: String,
        name: String,
    },
    Report {
        square_id: String,
        message_id: String,
    },
    Roles {
        square_id: String,
        members: Vec<OcMember>,
    },
    Post {
        chat_id: String,
        text: String,
        mention: MessageMention,
    },
    Sticker {
        package_id: String,
        sticker_id: String,
        #[serde(default = "default_sticker_text")]
        text: String,
        version: String,
        option: Option<String>,
    },
    Delete {
        chat_id: String,
        message_id: String,
    },
}
// 保存済みの旧Outboxだけは従来の代替文を維持する。
fn default_sticker_text() -> String {
    "[スタンプ]".into()
}

impl OcRequest {
    pub fn is_read(&self) -> bool {
        matches!(
            self,
            Self::Context { .. }
                | Self::Reactions { .. }
                | Self::Member { .. }
                | Self::Inspect { .. }
                | Self::Chats { .. }
                | Self::JoinedChats { .. }
                | Self::Members { .. }
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OcMember {
    pub member_id: String,
    pub square_id: String,
    pub name: String,
    pub role: String,
    pub state: String,
    pub revision: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OcContext {
    pub square_id: String,
    pub chat_name: String,
    pub bot_member_id: String,
    pub bot_role: String,
    pub actor: OcMember,
    pub authority: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OcChat {
    pub chat_id: String,
    pub name: String,
    pub is_main: bool,
    #[serde(default)]
    pub square_id: String,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OcResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reaction: Option<OcReaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context: Option<OcContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub member: Option<OcMember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw_member_name: Option<String>,
    #[serde(default)]
    pub chats: Vec<OcChat>,
    #[serde(default)]
    pub members: Vec<OcMember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub continuation_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OcReaction {
    pub member_id: String,
    pub reaction_type: String,
    #[ts(type = "number")]
    pub updated_at_ms: i64,
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
    pub preparing_media: u32,
    pub querying_actions: u32,
    pub pending_logs: u32,
    pub pending_log_bytes: u32,
}
