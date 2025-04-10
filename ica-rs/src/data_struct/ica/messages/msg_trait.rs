use std::fmt::Display;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::MainStatus;
use crate::data_struct::ica::messages::{At, Message, NewMessage};
use crate::data_struct::ica::{MessageId, UserId};

impl Serialize for At {
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
    fn is_reply(&self) -> bool;
    fn is_from_self(&self) -> bool {
        let qq_id = MainStatus::global_ica_status().online_status.qqid;
        self.sender_id() == qq_id
    }
    fn msg_id(&self) -> &MessageId;
    fn sender_id(&self) -> UserId;
    fn sender_name(&self) -> &String;
    fn content(&self) -> &String;
    fn time(&self) -> &DateTime<chrono::Utc>;
    fn role(&self) -> &String;
    fn has_files(&self) -> bool;
    fn deleted(&self) -> bool;
    fn system(&self) -> bool;
    fn reveal(&self) -> bool;
    fn flash(&self) -> bool;
    fn title(&self) -> &String;
    fn anonymous_id(&self) -> Option<i64>;
    fn hide(&self) -> bool;
    fn bubble_id(&self) -> i64;
    fn subid(&self) -> i64;
}

impl MessageTrait for Message {
    fn is_reply(&self) -> bool { self.reply.is_some() }
    fn msg_id(&self) -> &MessageId { &self.msg_id }
    fn sender_id(&self) -> UserId { self.sender_id }
    fn sender_name(&self) -> &String { &self.sender_name }
    fn content(&self) -> &String { &self.content }
    fn time(&self) -> &DateTime<chrono::Utc> { &self.time }
    fn role(&self) -> &String { &self.role }
    fn has_files(&self) -> bool { !self.files.is_empty() }
    fn deleted(&self) -> bool { self.deleted }
    fn system(&self) -> bool { self.system }
    fn reveal(&self) -> bool { self.reveal }
    fn flash(&self) -> bool { self.flash }
    fn title(&self) -> &String { &self.title }
    fn anonymous_id(&self) -> Option<i64> { self.anonymous_id }
    fn hide(&self) -> bool { self.hide }
    fn bubble_id(&self) -> i64 { self.bubble_id }
    fn subid(&self) -> i64 { self.subid }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Message, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let value = JsonValue::deserialize(deserializer)?;
        Ok(Message::new_from_json(&value))
    }
}

impl Display for Message {
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
    fn is_reply(&self) -> bool { self.msg.reply.is_some() }
    fn msg_id(&self) -> &MessageId { &self.msg.msg_id }
    fn sender_id(&self) -> UserId { self.msg.sender_id }
    fn sender_name(&self) -> &String { &self.msg.sender_name }
    fn content(&self) -> &String { &self.msg.content }
    fn time(&self) -> &DateTime<chrono::Utc> { &self.msg.time }
    fn role(&self) -> &String { &self.msg.role }
    fn has_files(&self) -> bool { !self.msg.files.is_empty() }
    fn deleted(&self) -> bool { self.msg.deleted }
    fn system(&self) -> bool { self.msg.system }
    fn reveal(&self) -> bool { self.msg.reveal }
    fn flash(&self) -> bool { self.msg.flash }
    fn title(&self) -> &String { &self.msg.title }
    fn anonymous_id(&self) -> Option<i64> { self.msg.anonymous_id }
    fn hide(&self) -> bool { self.msg.hide }
    fn bubble_id(&self) -> i64 { self.msg.bubble_id }
    fn subid(&self) -> i64 { self.msg.subid }
}

impl Display for NewMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.msg.content.trim().is_empty() {
            write!(
                f,
                "{}|{}|{}|{}|{}",
                self.msg.msg_id,
                self.room_id,
                self.msg.sender_id,
                self.msg.sender_name,
                self.msg.content
            )
        } else if !self.msg.files.is_empty() {
            write!(
                f,
                "{}|{}|{}|{}|{:?}",
                self.msg.msg_id,
                self.room_id,
                self.msg.sender_id,
                self.msg.sender_name,
                self.msg.files[0]
            )
        } else {
            write!(
                f,
                "{}|{}|{}|{}|empty content & empty files",
                self.msg.msg_id, self.room_id, self.msg.sender_id, self.msg.sender_name
            )
        }
    }
}
