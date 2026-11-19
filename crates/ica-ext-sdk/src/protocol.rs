//! NDJSON v1 线协议类型；未知字段不会影响已有消息的解析。

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToPlugin {
    HelloAck(HelloAck),
    NewMessage(NewMessage),
    ConnectionState(ConnectionState),
    RoomsUpdated { rooms: Vec<Room> },
    Response(Response),
    Shutdown(Shutdown),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginToHost {
    Hello(Hello),
    SendMessage {
        req_id: u64,
        #[serde(flatten)]
        message: SendMessage,
    },
    GetNtPicUrl {
        req_id: u64,
        file_id: String,
    },
    RequestReload {
        req_id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub subscribe: Subscribe,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Subscribe {
    pub commands: Vec<String>,
    pub all_messages: bool,
    pub system_messages: bool,
    pub connection_events: bool,
    pub rooms_events: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    pub client_id: String,
    pub self_id: Option<i64>,
    pub admin_list: Vec<i64>,
    pub connection: ConnectionState,
    pub rooms: Vec<Room>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionState {
    pub ready: bool,
    pub online: bool,
    pub account: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub room_id: i64,
    pub room_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessage {
    pub room_id: i64,
    pub msg_id: String,
    pub sender_id: i64,
    pub sender_name: String,
    pub role: String,
    pub content: String,
    pub time_ms: i64,
    pub is_from_self: bool,
    pub is_admin: bool,
    pub is_reply: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyMessage>,
    pub files: Vec<MessageFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyMessage {
    pub msg_id: String,
    pub sender_id: Option<i64>,
    pub sender_name: String,
    pub content: String,
    pub files: Vec<MessageFile>,
}

/// 保留 oicq Bridge 附件字段，引用中的附件可通过 fid 向宿主换取下载地址。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageFile {
    #[serde(rename = "type")]
    pub file_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(rename = "isFace", skip_serializing_if = "Option::is_none")]
    pub is_face: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendMessage {
    pub room_id: i64,
    /// 被引用的消息 ID，限当前房间。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    pub mime: String,
    #[serde(default)]
    pub as_sticker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub req_id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Shutdown {
    pub reason: ShutdownReason,
    pub grace_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    Reload,
    Disable,
    HostExit,
}
