use crate::data_struct::ica::messages::{At, LastMessage};
use crate::data_struct::ica::RoomId;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value as JsonValue};

/// export default interface Room {
///     roomId: number
///     roomName: string
///     index: number
///     unreadCount: number
///     priority: 1 | 2 | 3 | 4 | 5
///     utime: number
///     users:
///         | [{ _id: 1; username: '1' }, { _id: 2; username: '2' }]
///         | [{ _id: 1; username: '1' }, { _id: 2; username: '2' }, { _id: 3; username: '3' }]
///     at?: boolean | 'all'
///     lastMessage: LastMessage
///     autoDownload?: boolean
///     downloadPath?: string
/// }
#[derive(Debug, Clone)]
pub struct Room {
    pub room_id: RoomId,
    pub room_name: String,
    pub index: i64,
    pub unread_count: u64,
    pub priority: u8,
    pub utime: i64,
    /// 我严重怀疑是脱裤子放屁
    /// 历史遗留啊,那没事了()
    // pub users: JsonValue,
    pub at: At,
    pub last_message: LastMessage,
    // 这俩都没啥用
    // pub auto_download: Option<String>,
    // pub download_path: Option<String>,
}

impl Room {
    pub fn new_from_json(raw_json: &JsonValue) -> Self {
        let mut parse_json = raw_json.clone();
        // 手动 patch 一下 roomId
        // ica issue: https://github.com/Icalingua-plus-plus/Icalingua-plus-plus/issues/793
        if parse_json.get("roomId").is_none_or(|id| id.is_null()) {
            use tracing::warn;
            warn!("Room::new_from_json roomId is None, patching it to -1, raw: {:?}", raw_json);
            parse_json["roomId"] = JsonValue::Number(Number::from(-1));
        }
        // 现在 fix 了

        let inner = match serde_json::from_value::<InnerRoom>(parse_json) {
            Ok(data) => data,
            Err(e) => {
                panic!("Room::new_from_json error: {}, raw: {:#?}", e, raw_json);
            }
        };
        let at = At::new_from_json(&raw_json["at"]);
        Self {
            room_id: inner.room_id,
            room_name: inner.room_name,
            index: inner.index,
            unread_count: inner.unread_count,
            priority: inner.priority,
            utime: inner.utime,
            // users: inner.users,
            at,
            last_message: inner.last_message,
            // download_path: inner.download_path,
        }
    }
}

fn room_id_default() -> RoomId { -1 }

#[derive(Debug, Clone, Deserialize, Serialize)]
struct InnerRoom {
    #[serde(rename = "roomId", default = "room_id_default")]
    pub room_id: RoomId,
    #[serde(rename = "roomName")]
    pub room_name: String,
    #[serde(rename = "index")]
    pub index: i64,
    #[serde(rename = "unreadCount")]
    pub unread_count: u64,
    #[serde(rename = "priority")]
    pub priority: u8,
    #[serde(rename = "utime")]
    pub utime: i64,
    #[serde(rename = "users")]
    pub users: JsonValue,
    // 忽略 at
    #[serde(rename = "lastMessage")]
    pub last_message: LastMessage,
    // 这俩都没啥用
    // #[serde(rename = "autoDownload")]
    // pub auto_download: Option<String>,
    // #[serde(rename = "downloadPath")]
    // pub download_path: Option<String>,
}
