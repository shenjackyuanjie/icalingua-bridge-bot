//! Socket.IO/oicq 推送到本地状态的业务转换；与网络连接和 Python 调度分离。

#[cfg(test)]
#[path = "tests/event_state.rs"]
mod tests;

use serde::Deserialize;
use serde::de::Error as _;
use serde_json::{Value, json};

use crate::data_struct::ica::all_rooms::Room;
use crate::data_struct::ica::messages::{At, LastMessage, Message, NewMessage};
use crate::status::ica::MainStatus;

#[derive(Debug)]
pub struct LoadedBatch {
    pub accepted: usize,
    pub rejected: usize,
    pub cached: u64,
}

#[derive(Debug)]
pub struct MessageUpdate {
    pub room_id: i64,
    pub message_id: String,
    pub patch: Value,
    pub first_deletion: bool,
}

/// 只接受真实房间和合法 Message；畸形实时推送返回可概括的错误，不 panic。
pub fn decode_add_message(value: &Value) -> Result<NewMessage, serde_json::Error> {
    let room_id = room_id(value.get("roomId"))?;
    let raw = value.get("message").ok_or_else(|| invalid("缺少 Message"))?;
    Ok(NewMessage::new(room_id, Message::new_from_json(raw)?))
}

/// 部分坏消息不阻断同一批次其他合法消息；调用者不把结果投递为新消息。
pub fn decode_messages(values: &[Value]) -> (Vec<Message>, usize) {
    let mut messages = Vec::with_capacity(values.len());
    let mut rejected = 0;
    for value in values {
        match Message::new_from_json(value) {
            Ok(message) => messages.push(message),
            Err(_) => rejected += 1,
        }
    }
    (messages, rejected)
}

pub fn store_messages(status: &mut MainStatus, room_id: i64, values: &[Value]) -> LoadedBatch {
    let (messages, rejected) = decode_messages(values);
    let accepted = status.store_loaded_messages(room_id, messages, false);
    LoadedBatch {
        accepted,
        rejected,
        cached: status.loaded_messages_count_for(room_id),
    }
}

pub fn set_messages(
    status: &mut MainStatus,
    value: &Value,
) -> Result<LoadedBatch, serde_json::Error> {
    let room_id = room_id(value.get("roomId"))?;
    let values = value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("messages 必须是数组"))?;
    Ok(store_messages(status, room_id, values))
}

pub fn renew_message(
    status: &mut MainStatus,
    value: &Value,
) -> Result<MessageUpdate, serde_json::Error> {
    let room_id = room_id(value.get("roomId"))?;
    let message_id = message_id(value.get("messageId"))?;
    let patch = value
        .get("message")
        .filter(|patch| patch.is_object())
        .ok_or_else(|| invalid("message patch 必须是对象"))?;
    // 未加载的消息也验证补丁类型，但不创建假消息或触发实时新消息。
    let mut validation = Message::new_from_json(&json!({"_id": message_id}))?;
    validation.apply_patch(patch)?;
    status.patch_loaded_message(room_id, &message_id, patch)?;
    let first_deletion = patch.get("deleted").and_then(Value::as_bool) == Some(true)
        && status.mark_message_deleted(&message_id);
    Ok(MessageUpdate {
        room_id,
        message_id,
        patch: patch.clone(),
        first_deletion,
    })
}

pub fn delete_message(
    status: &mut MainStatus,
    value: &Value,
) -> Result<Option<String>, serde_json::Error> {
    let message_id = message_id(Some(value))?;
    Ok(status.mark_message_deleted(&message_id).then_some(message_id))
}

pub fn renew_message_url(
    status: &mut MainStatus,
    value: &Value,
) -> Result<Vec<(i64, String, Value)>, serde_json::Error> {
    let message_id = message_id(value.get("messageId"))?;
    let url = value
        .get("URL")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| invalid("URL 必须是非空字符串"))?;
    Ok(status
        .renew_loaded_message_url(&message_id, url)
        .into_iter()
        .map(|(room_id, patch)| (room_id, message_id.clone(), patch))
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeRoom {
    room_id: i64,
    room_name: String,
    index: i64,
    unread_count: u64,
    priority: u8,
    utime: i64,
    last_message: LastMessage,
    #[serde(default)]
    at: Value,
}

fn decode_room(value: &Value) -> Result<Room, serde_json::Error> {
    let fields: BridgeRoom = serde_json::from_value(value.clone())?;
    if fields.room_id == 0 || !(1..=5).contains(&fields.priority) {
        return Err(invalid("Room 的房间或优先级无效"));
    }
    let at = match fields.at {
        Value::Null => At::None,
        Value::Bool(value) => At::Bool(value),
        Value::String(value) if value == "all" => At::All,
        _ => return Err(invalid("Room.at 类型无效")),
    };
    Ok(Room {
        room_id: fields.room_id,
        room_name: fields.room_name,
        index: fields.index,
        unread_count: fields.unread_count,
        priority: fields.priority,
        utime: fields.utime,
        at,
        last_message: fields.last_message,
    })
}

/// 全量房间快照先完整验证，再替换；坏条目不误清其他已加载房间。
pub fn set_all_rooms(status: &mut MainStatus, value: &Value) -> Result<usize, serde_json::Error> {
    let values = value.as_array().ok_or_else(|| invalid("rooms 必须是数组"))?;
    let rooms: Vec<_> = values.iter().map(decode_room).collect::<Result<_, _>>()?;
    status.update_rooms(rooms);
    status.room_data = values
        .iter()
        .filter_map(|value| {
            value
                .get("roomId")
                .and_then(Value::as_i64)
                .map(|room_id| (room_id, value.clone()))
        })
        .collect();
    Ok(status.rooms.len())
}

/// 单房间更新按原始字典合并，保留未覆盖的 users、下载配置及扩展字段。
pub fn update_room(status: &mut MainStatus, patch: &Value) -> Result<(), serde_json::Error> {
    let room_id = room_id(patch.get("roomId"))?;
    let fields = patch.as_object().ok_or_else(|| invalid("Room 必须是对象"))?;
    let mut value = status.room_data.get(&room_id).cloned().unwrap_or_else(|| json!({}));
    value
        .as_object_mut()
        .ok_or_else(|| invalid("Room 快照必须是对象"))?
        .extend(fields.clone());
    let room = decode_room(&value)?;
    if let Some(current) = status.rooms.iter_mut().find(|current| current.room_id == room_id) {
        *current = room;
    } else {
        status.rooms.push(room);
    }
    status.room_data.insert(room_id, value);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeChatGroup {
    name: String,
    index: i64,
    rooms: Vec<i64>,
    include_all_personal: Option<bool>,
}

pub fn set_all_chat_groups(
    status: &mut MainStatus,
    value: &Value,
) -> Result<usize, serde_json::Error> {
    let values = value.as_array().ok_or_else(|| invalid("chat groups 必须是数组"))?;
    for value in values {
        let group: BridgeChatGroup = serde_json::from_value(value.clone())?;
        // 读取已知字段验证当前契约，保存原始字典以保留扩展字段。
        let _ = (group.name, group.index, group.rooms, group.include_all_personal);
    }
    status.chat_groups = values.clone();
    Ok(values.len())
}

pub fn sync_read(status: &mut MainStatus, value: &Value) -> Result<i64, serde_json::Error> {
    let room_id = room_id(Some(value))?;
    status.sync_read(room_id);
    Ok(room_id)
}

fn room_id(value: Option<&Value>) -> Result<i64, serde_json::Error> {
    value
        .and_then(Value::as_i64)
        .filter(|room_id| *room_id != 0)
        .ok_or_else(|| invalid("roomId 必须是非零整数"))
}

fn message_id(value: Option<&Value>) -> Result<String, serde_json::Error> {
    match value {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        _ => Err(invalid("messageId 必须是非空字符串或数字")),
    }
}

fn invalid(message: &'static str) -> serde_json::Error { serde_json::Error::custom(message) }
