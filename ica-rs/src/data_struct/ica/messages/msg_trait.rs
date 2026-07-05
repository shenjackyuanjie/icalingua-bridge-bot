//! Icalingua 消息的统一访问、显示和反序列化实现。

use std::fmt::Display;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::MainStatus;
use crate::data_struct::ica::messages::{At, Message, NewMessage};
use crate::data_struct::ica::{MessageId, UserId};

impl Serialize for At {
    /// 将当前值序列化到指定序列化器。
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match self {
            At::All => serializer.serialize_str("all"),
            At::Bool(b) => serializer.serialize_bool(*b),
            At::None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for At {
    /// 从指定反序列化器构造当前值。
    fn deserialize<D>(deserializer: D) -> Result<At, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let value = JsonValue::deserialize(deserializer)?;
        Ok(At::new_from_json(&value))
    }
}

#[allow(unused)]
pub trait MessageTrait {
    /// 判断当前值是否满足 `reply` 条件。
    fn is_reply(&self) -> bool;
    /// 判断当前值是否满足 `from_self` 条件。
    fn is_from_self(&self) -> bool {
        let qq_id = MainStatus::global_ica_status().online_status.qqid;
        self.sender_id() == qq_id
    }
    /// 返回消息 ID。
    fn msg_id(&self) -> &MessageId;
    /// 返回消息发送者 ID。
    fn sender_id(&self) -> UserId;
    /// 返回消息发送者名称。
    fn sender_name(&self) -> &String;
    /// 返回消息文本内容。
    fn content(&self) -> &String;
    /// 返回消息时间。
    fn time(&self) -> &DateTime<chrono::Utc>;
    /// 返回消息发送者角色。
    fn role(&self) -> &String;
    /// 判断当前值是否包含 `files` 数据。
    fn has_files(&self) -> bool;
    /// 返回消息是否已删除。
    fn deleted(&self) -> bool;
    /// 返回消息是否为系统消息。
    fn system(&self) -> bool;
    /// 返回消息是否处于显示状态。
    fn reveal(&self) -> bool;
    /// 返回消息是否为闪照。
    fn flash(&self) -> bool;
    /// 返回消息标题。
    fn title(&self) -> &String;
    /// 返回匿名发送者 ID。
    fn anonymous_id(&self) -> Option<i64>;
    /// 返回消息是否处于隐藏状态。
    fn hide(&self) -> bool;
    /// 返回消息气泡 ID。
    fn bubble_id(&self) -> i64;
    /// 返回消息子 ID。
    fn subid(&self) -> i64;
}

impl MessageTrait for Message {
    /// 判断当前值是否满足 `reply` 条件。
    fn is_reply(&self) -> bool { self.reply.is_some() }
    /// 返回消息 ID。
    fn msg_id(&self) -> &MessageId { &self.msg_id }
    /// 返回消息发送者 ID。
    fn sender_id(&self) -> UserId { self.sender_id }
    /// 返回消息发送者名称。
    fn sender_name(&self) -> &String { &self.sender_name }
    /// 返回消息文本内容。
    fn content(&self) -> &String { &self.content }
    /// 返回消息时间。
    fn time(&self) -> &DateTime<chrono::Utc> { &self.time }
    /// 返回消息发送者角色。
    fn role(&self) -> &String { &self.role }
    /// 判断当前值是否包含 `files` 数据。
    fn has_files(&self) -> bool { !self.files.is_empty() }
    /// 返回消息是否已删除。
    fn deleted(&self) -> bool { self.deleted }
    /// 返回消息是否为系统消息。
    fn system(&self) -> bool { self.system }
    /// 返回消息是否处于显示状态。
    fn reveal(&self) -> bool { self.reveal }
    /// 返回消息是否为闪照。
    fn flash(&self) -> bool { self.flash }
    /// 返回消息标题。
    fn title(&self) -> &String { &self.title }
    /// 返回匿名发送者 ID。
    fn anonymous_id(&self) -> Option<i64> { self.anonymous_id }
    /// 返回消息是否处于隐藏状态。
    fn hide(&self) -> bool { self.hide }
    /// 返回消息气泡 ID。
    fn bubble_id(&self) -> i64 { self.bubble_id }
    /// 返回消息子 ID。
    fn subid(&self) -> i64 { self.subid }
}

impl<'de> Deserialize<'de> for Message {
    /// 从指定反序列化器构造当前值。
    fn deserialize<D>(deserializer: D) -> Result<Message, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let value = JsonValue::deserialize(deserializer)?;
        Ok(Message::new_from_json(&value))
    }
}

impl Display for Message {
    /// 将当前值写入格式化输出。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.content.is_empty() && !self.content.trim().is_empty() {
            write!(f, "{}|{}|{}|{}", self.msg_id(), self.sender_id, self.sender_name, self.content)
        } else if !self.files.is_empty() {
            write!(
                f,
                "{}|{}|{}|{:?}",
                self.msg_id(),
                self.sender_id,
                self.sender_name,
                self.files[0].name
            )
        } else {
            write!(
                f,
                "{}|{}|{}|empty content & empty files",
                self.msg_id(),
                self.sender_id,
                self.sender_name
            )
        }
    }
}

impl MessageTrait for NewMessage {
    /// 判断当前值是否满足 `reply` 条件。
    fn is_reply(&self) -> bool { self.msg.reply.is_some() }
    /// 返回消息 ID。
    fn msg_id(&self) -> &MessageId { &self.msg.msg_id }
    /// 返回消息发送者 ID。
    fn sender_id(&self) -> UserId { self.msg.sender_id }
    /// 返回消息发送者名称。
    fn sender_name(&self) -> &String { &self.msg.sender_name }
    /// 返回消息文本内容。
    fn content(&self) -> &String { &self.msg.content }
    /// 返回消息时间。
    fn time(&self) -> &DateTime<chrono::Utc> { &self.msg.time }
    /// 返回消息发送者角色。
    fn role(&self) -> &String { &self.msg.role }
    /// 判断当前值是否包含 `files` 数据。
    fn has_files(&self) -> bool { !self.msg.files.is_empty() }
    /// 返回消息是否已删除。
    fn deleted(&self) -> bool { self.msg.deleted }
    /// 返回消息是否为系统消息。
    fn system(&self) -> bool { self.msg.system }
    /// 返回消息是否处于显示状态。
    fn reveal(&self) -> bool { self.msg.reveal }
    /// 返回消息是否为闪照。
    fn flash(&self) -> bool { self.msg.flash }
    /// 返回消息标题。
    fn title(&self) -> &String { &self.msg.title }
    /// 返回匿名发送者 ID。
    fn anonymous_id(&self) -> Option<i64> { self.msg.anonymous_id }
    /// 返回消息是否处于隐藏状态。
    fn hide(&self) -> bool { self.msg.hide }
    /// 返回消息气泡 ID。
    fn bubble_id(&self) -> i64 { self.msg.bubble_id }
    /// 返回消息子 ID。
    fn subid(&self) -> i64 { self.msg.subid }
}

impl Display for NewMessage {
    /// 将当前值写入格式化输出。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.msg.content.trim().is_empty() {
            write!(
                f,
                "{}[{}]({}){}|{}",
                self.msg.msg_id,
                self.room_id,
                self.msg.sender_id,
                self.msg.sender_name,
                self.msg.content
            )
        } else if !self.msg.files.is_empty() {
            write!(
                f,
                "{}[{}]({}){}|{:?}",
                self.msg.msg_id,
                self.room_id,
                self.msg.sender_id,
                self.msg.sender_name,
                self.msg.files[0]
            )
        } else {
            write!(
                f,
                "{}[{}]({}){}|empty content & empty files",
                self.msg.msg_id, self.room_id, self.msg.sender_id, self.msg.sender_name
            )
        }
    }
}
