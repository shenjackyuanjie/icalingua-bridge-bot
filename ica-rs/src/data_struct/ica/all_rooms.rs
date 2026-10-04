//! Icalingua 房间和好友、群申请数据结构。

use crate::data_struct::ica::messages::{At, LastMessage, SendMessage};
use crate::data_struct::ica::{RoomId, UserId};

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
    /// 创建并初始化对应的数据结构。
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
                panic!("Room::new_from_json error: {e}, raw: {raw_json:#?}");
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
    /// 创建并初始化对应的数据结构。
    pub fn new_message_to(&self, content: String) -> SendMessage {
        SendMessage::new(content, self.room_id, None)
    }
}

/// 返回缺失房间 ID 时使用的默认值。
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

/// ```json
/// {
///  "comment": "问题：从哪里了解到的本群\n答案：aaa",
///  "flag": "e4cd5a892ba34bed063196a0cc47a8",
///  "group_id": xxxxx,
///  "group_name": "Nuitka 和 Python 打包",
///  "nickname": "jashcken",
///  "post_type": "request",
///  "request_type": "group",
///  "self_id": 45620725,
///  "sub_type": "add",
///  "time": 1743372872,
///  "tips": "",
///  "user_id": 3838663305
/// }
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JoinRequestRoom {
    /// 问题+答案
    #[serde(default)]
    pub comment: String,
    /// 好友申请没有群号，不使用虚构的 0 或负数房间号。
    pub group_id: Option<RoomId>,
    #[serde(default)]
    pub group_name: String,
    pub user_id: UserId,
    #[serde(default)]
    pub nickname: String,

    // 剩下的应该没用了……吧?
    pub request_type: String,
    pub post_type: String,
    pub sub_type: String,
    pub time: i64,
    #[serde(default)]
    pub tips: String,
    pub flag: String,
    #[serde(default)]
    pub source: String,
    pub self_id: Option<UserId>,
    pub age: Option<u32>,
    pub sex: Option<String>,
}

#[cfg(test)]
mod request_tests {
    use super::JoinRequestRoom;
    use serde_json::json;

    #[test]
    fn friend_request_without_group_fields_reaches_the_shared_hook_model() {
        let request: JoinRequestRoom = serde_json::from_value(json!({
            "age": 0, "comment": "来自群聊的好友申请", "flag": "test-friend-flag",
            "nickname": "测试用户", "post_type": "request", "request_type": "friend",
            "self_id": 10, "sex": "male", "source": "QQ群", "sub_type": "add",
            "time": 1_791_114_907, "user_id": 20
        }))
        .unwrap();
        assert_eq!(request.request_type, "friend");
        assert_eq!(request.group_id, None);
        assert!(request.group_name.is_empty());
        assert!(request.tips.is_empty());
        assert_eq!(request.source, "QQ群");
    }

    #[test]
    fn group_add_and_invite_keep_the_positive_group_id() {
        for sub_type in ["add", "invite"] {
            let request: JoinRequestRoom = serde_json::from_value(json!({
                "group_id": 30, "group_name": "测试群", "user_id": 20,
                "request_type": "group", "post_type": "request", "sub_type": sub_type,
                "time": 1, "flag": "test-group-flag"
            }))
            .unwrap();
            assert_eq!(request.group_id, Some(30));
            assert_eq!(request.sub_type, sub_type);
        }
    }

    #[test]
    fn missing_request_identity_is_not_silently_defaulted() {
        assert!(
            serde_json::from_value::<JoinRequestRoom>(json!({
                "request_type": "friend", "post_type": "request", "sub_type": "add", "time": 1
            }))
            .is_err()
        );
    }
}
