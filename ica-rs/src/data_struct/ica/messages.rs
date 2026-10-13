//! Icalingua bridge 收发消息、回复和消息内容数据结构。

use crate::data_struct::ica::files::MessageFile;
use crate::data_struct::ica::{MessageId, RoomId, UserId};

use chrono::DateTime;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

#[cfg(test)]
#[path = "messages/tests/bridge_contract.rs"]
mod bridge_contract_tests;

/// 加载 `msg_trait` 子模块。
pub mod msg_trait;
#[allow(unused)]
/// 加载 `raw` 子模块。
pub mod raw;

pub use msg_trait::MessageTrait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    All,
    Bool(bool),
    /// dummy
    None,
}

impl At {
    #[inline]
    /// 创建并初始化对应的数据结构。
    pub fn new_from_json(json: &JsonValue) -> Self {
        match json {
            JsonValue::Bool(b) => Self::Bool(*b),
            #[allow(non_snake_case)]
            JsonValue::String(_I_dont_Care) => Self::All,
            _ => Self::None,
        }
    }
}

/*export default interface LastMessage {
    content?: string
    timestamp?: string
    username?: string
    userId?: number
}
 */
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LastMessage {
    pub content: Option<String>,
    pub timestamp: Option<String>,
    pub username: Option<String>,
    // 因为这玩意可能返回 raw buffer, 所以先不解析了
    // #[serde(rename = "userId")]
    // pub user_id: Option<i64>,
}

/// 回复摘要保留原有访问字段，同时保留完整引用消息，避免 senderId/媒体/嵌套回复丢失。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyMessage {
    pub msg_id: String,
    pub content: String,
    pub files: JsonValue,
    pub sender_name: String,
    pub raw_data: JsonValue,
}

impl ReplyMessage {
    pub fn to_bridge_value(&self) -> JsonValue {
        let mut value = self.raw_data.as_object().cloned().unwrap_or_default();
        value.insert("_id".into(), json!(self.msg_id));
        value.insert("content".into(), json!(self.content));
        value.insert("files".into(), self.files.clone());
        value.insert("username".into(), json!(self.sender_name));
        JsonValue::Object(value)
    }
}

impl Serialize for ReplyMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_bridge_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReplyMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = JsonValue::deserialize(deserializer)?;
        Message::new_from_json(&value)
            .map(|message| message.as_reply())
            .map_err(serde::de::Error::custom)
    }
}

/*
export default interface Message {
    _id: string | number
    senderId?: number
    username: string
    content: string
    code?: string
    timestamp?: string
    date?: string
    role?: string
    file?: MessageFile
    files: MessageFile[]
    time?: number
    replyMessage?: Message
    at?: boolean | 'all'
    deleted?: boolean
    system?: boolean
    mirai?: MessageMirai
    reveal?: boolean
    flash?: boolean
    title?: string
    anonymousId?: number
    anonymousflag?: string
    hide?: boolean
    bubble_id?: number
    subid?: number
    head_img?: string
}*/
/// {"message": {"_id":"idddddd","anonymousId":null,"anonymousflag":null,"bubble_id":0,"content":"test","date":"2024/02/18","files":[],"role":"admin","senderId":123456,"subid":1,"time":1708267062000_i64,"timestamp":"22:37:42","title":"索引管理员","username":"shenjack"},"roomId":-123456}
#[derive(Debug, Clone)]
pub struct Message {
    // /// 房间 id
    // pub room_id: RoomId,
    /// 消息 id
    pub msg_id: MessageId,
    /// 发送者 id
    pub sender_id: UserId,
    /// 发送者名字
    pub sender_name: String,
    /// 消息内容
    pub content: String,
    /// xml / json 内容
    pub code: JsonValue,
    /// 消息时间
    pub time: DateTime<chrono::Utc>,
    /// 身份
    pub role: String,
    /// 文件
    pub files: Vec<MessageFile>,
    /// 回复的消息
    pub reply: Option<ReplyMessage>,
    /// At
    pub at: At,
    /// 是否已撤回
    pub deleted: bool,
    /// 是否是系统消息
    pub system: bool,
    /// mirai?
    pub mirai: JsonValue,
    /// reveal ?
    pub reveal: bool,
    /// flash
    pub flash: bool,
    /// "群主授予的头衔"
    pub title: String,
    /// anonymous id
    pub anonymous_id: Option<i64>,
    /// 是否已被隐藏
    pub hide: bool,
    /// 气泡 id
    pub bubble_id: i64,
    /// 子? id
    pub subid: i64,
    /// 头像 img?
    pub head_img: JsonValue,
    /// 协议原始节点链；不等同规范化 Message。没有节点链时为 Null。
    pub raw_msg: JsonValue,
    /// 完整规范化 Message 输入，保留未知字段，供插件与消息变更消费。
    pub raw_data: JsonValue,
    /// 旧版单附件字段，解析时也会补入 files。
    pub file: Option<MessageFile>,
    /// 跨房间查询附带的消息来源。
    pub room_id: Option<RoomId>,
    pub room_name: Option<String>,
    pub timestamp: Option<String>,
    pub date: Option<String>,
    pub anonymous_flag: Option<String>,
    pub markdown: bool,
    pub button_rows: JsonValue,
    pub recall_info: Option<String>,
}

/// 可缺省的已知字段使用 Option，显式的错误类型仍会返回反序列化错误。
#[derive(Deserialize)]
struct BridgeMessageFields {
    #[serde(rename = "_id")]
    msg_id: JsonValue,
    #[serde(rename = "senderId")]
    sender_id: Option<UserId>,
    username: Option<String>,
    content: Option<String>,
    #[serde(default)]
    code: JsonValue,
    time: Option<i64>,
    role: Option<String>,
    files: Option<Vec<MessageFile>>,
    file: Option<MessageFile>,
    #[serde(rename = "replyMessage")]
    reply: Option<ReplyMessage>,
    #[serde(default)]
    at: JsonValue,
    deleted: Option<bool>,
    system: Option<bool>,
    #[serde(default)]
    mirai: JsonValue,
    reveal: Option<bool>,
    flash: Option<bool>,
    title: Option<String>,
    #[serde(rename = "anonymousId")]
    anonymous_id: Option<i64>,
    hide: Option<bool>,
    bubble_id: Option<i64>,
    subid: Option<i64>,
    #[serde(default)]
    head_img: JsonValue,
    #[serde(rename = "roomId")]
    room_id: Option<RoomId>,
    #[serde(rename = "_roomName")]
    room_name: Option<String>,
    timestamp: Option<String>,
    date: Option<String>,
    #[serde(rename = "anonymousflag")]
    anonymous_flag: Option<String>,
    markdown: Option<bool>,
    button_rows: Option<Vec<Vec<BridgeMessageButton>>>,
    #[serde(rename = "recallInfo")]
    recall_info: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct BridgeMessageButton {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    visited_label: Option<String>,
    url: String,
    #[serde(flatten)]
    extra: serde_json::Map<String, JsonValue>,
}

impl Message {
    /// 从 oicq 规范化 Message 构造模型；畸形字段返回错误，调用者可跳过该条消息。
    pub fn new_from_json(json: &JsonValue) -> Result<Self, serde_json::Error> {
        if !json.is_object() {
            return Err(serde_json::Error::custom("Message 必须是对象"));
        }
        let fields: BridgeMessageFields = serde_json::from_value(json.clone())?;
        let msg_id = match fields.msg_id {
            JsonValue::String(value) if !value.is_empty() => value,
            JsonValue::Number(value) => value.to_string(),
            _ => return Err(serde_json::Error::custom("Message._id 必须是非空字符串或数字")),
        };
        let time = match fields.time {
            Some(value) => parse_bridge_timestamp(value)
                .ok_or_else(|| serde_json::Error::custom("Message.time 超出可表示时间范围"))?,
            None => chrono::Utc::now(),
        };
        let at = match &fields.at {
            JsonValue::Null => At::None,
            JsonValue::Bool(value) => At::Bool(*value),
            JsonValue::String(value) if value == "all" => At::All,
            _ => return Err(serde_json::Error::custom("Message.at 必须是 boolean 或 'all'")),
        };
        let mut files = fields.files.unwrap_or_default();
        if let Some(file) = &fields.file
            && !files.contains(file)
        {
            files.push(file.clone());
        }
        let raw_msg = json
            .get("message")
            .filter(|value| raw::validate_node_chain(value).is_ok())
            .cloned()
            .unwrap_or(JsonValue::Null);
        Ok(Self {
            msg_id,
            sender_id: fields.sender_id.unwrap_or(-1),
            sender_name: fields.username.unwrap_or_default(),
            content: fields.content.unwrap_or_default(),
            code: fields.code,
            time,
            role: fields.role.unwrap_or_else(|| "unknown".into()),
            files,
            reply: fields.reply,
            at,
            deleted: fields.deleted.unwrap_or(false),
            system: fields.system.unwrap_or(false),
            mirai: fields.mirai,
            reveal: fields.reveal.unwrap_or(false),
            flash: fields.flash.unwrap_or(false),
            title: fields.title.unwrap_or_default(),
            anonymous_id: fields.anonymous_id,
            hide: fields.hide.unwrap_or(false),
            bubble_id: fields.bubble_id.unwrap_or(1),
            subid: fields.subid.unwrap_or(1),
            head_img: fields.head_img,
            raw_msg,
            raw_data: json.clone(),
            file: fields.file,
            room_id: fields.room_id,
            room_name: fields.room_name,
            timestamp: fields.timestamp,
            date: fields.date,
            anonymous_flag: fields.anonymous_flag,
            markdown: fields.markdown.unwrap_or(false),
            button_rows: json!(fields.button_rows.unwrap_or_default()),
            recall_info: fields.recall_info,
        })
    }

    /// 输出当前模型的完整规范化 Message；保留未知字段，不混入发送用的 raw 节点链。
    pub fn to_bridge_value(&self) -> JsonValue {
        let mut value = self.raw_data.as_object().cloned().unwrap_or_default();
        value.insert("_id".into(), json!(self.msg_id));
        if self.sender_id != -1 {
            value.insert("senderId".into(), json!(self.sender_id));
        } else {
            value.remove("senderId");
        }
        value.insert("username".into(), json!(self.sender_name));
        value.insert("content".into(), json!(self.content));
        value.insert("code".into(), self.code.clone());
        value.insert("time".into(), json!(self.time.timestamp_millis()));
        value.insert("role".into(), json!(self.role));
        value.insert("files".into(), json!(self.files));
        value.insert("replyMessage".into(), json!(self.reply));
        value.insert("at".into(), json!(self.at));
        value.insert("deleted".into(), json!(self.deleted));
        value.insert("system".into(), json!(self.system));
        value.insert("mirai".into(), self.mirai.clone());
        value.insert("reveal".into(), json!(self.reveal));
        value.insert("flash".into(), json!(self.flash));
        value.insert("title".into(), json!(self.title));
        value.insert("anonymousId".into(), json!(self.anonymous_id));
        value.insert("hide".into(), json!(self.hide));
        value.insert("bubble_id".into(), json!(self.bubble_id));
        value.insert("subid".into(), json!(self.subid));
        value.insert("head_img".into(), self.head_img.clone());
        value.insert("file".into(), json!(self.file));
        value.insert("roomId".into(), json!(self.room_id));
        value.insert("_roomName".into(), json!(self.room_name));
        value.insert("timestamp".into(), json!(self.timestamp));
        value.insert("date".into(), json!(self.date));
        value.insert("anonymousflag".into(), json!(self.anonymous_flag));
        value.insert("markdown".into(), json!(self.markdown));
        value.insert("button_rows".into(), self.button_rows.clone());
        value.insert("recallInfo".into(), json!(self.recall_info));
        JsonValue::Object(value)
    }

    /// 合并 Bridge Partial<Message>；验证成功后一次替换，错误时保留旧模型。
    pub fn apply_patch(&mut self, patch: &JsonValue) -> Result<(), serde_json::Error> {
        let patch = patch
            .as_object()
            .ok_or_else(|| serde_json::Error::custom("Message patch 必须是对象"))?;
        if let Some(id) = patch.get("_id") {
            let matches = match id {
                JsonValue::String(value) => value == &self.msg_id,
                JsonValue::Number(value) => value.to_string() == self.msg_id,
                _ => false,
            };
            if !matches {
                return Err(serde_json::Error::custom("Message patch 不能改变消息 ID"));
            }
        }
        let mut value = self.to_bridge_value();
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde_json::Error::custom("Message 必须是对象"))?;
        // files 是完整的新列表；没有同时提供 file 时，旧单附件不能再次补入列表。
        if patch.contains_key("files") && !patch.contains_key("file") {
            object.remove("file");
        }
        object.extend(patch.clone());
        let updated = Self::new_from_json(&value)?;
        *self = updated;
        Ok(())
    }

    /// 向 Python 插件日志输出消息。
    pub fn output(&self) -> String {
        format!(
            // >10 >10 >15
            // >10 >15
            "{:>12}|{:<20}|{}",
            self.sender_id, self.sender_name, self.content
        )
    }

    /// 作为回复消息使用
    pub fn as_reply(&self) -> ReplyMessage {
        ReplyMessage {
            msg_id: self.msg_id.clone(),
            content: self.content.clone(),
            files: json!(self.files),
            sender_name: self.sender_name.clone(),
            raw_data: self.to_bridge_value(),
        }
    }

    /// 获取回复
    pub fn get_reply(&self) -> Option<&ReplyMessage> { self.reply.as_ref() }

    /// 返回 `reply_mut` 对应的数据。
    pub fn get_reply_mut(&mut self) -> Option<&mut ReplyMessage> { self.reply.as_mut() }
}

/// 按 bridge 时间戳的数量级兼容秒、毫秒和微秒。
pub(crate) fn parse_bridge_timestamp(value: i64) -> Option<DateTime<chrono::Utc>> {
    let magnitude = value.unsigned_abs();
    if magnitude >= 100_000_000_000_000 {
        DateTime::from_timestamp_micros(value)
    } else if magnitude >= 100_000_000_000 {
        DateTime::from_timestamp_millis(value)
    } else {
        DateTime::from_timestamp(value, 0)
    }
}

#[cfg(test)]
mod timestamp_tests {
    use super::parse_bridge_timestamp;

    #[test]
    fn accepts_seconds_milliseconds_and_microseconds() {
        let expected = 1_710_000_000;
        for value in [expected, expected * 1_000, expected * 1_000_000] {
            assert_eq!(parse_bridge_timestamp(value).unwrap().timestamp(), expected);
        }
    }

    #[test]
    fn rejects_overflowing_timestamp() {
        assert!(parse_bridge_timestamp(i64::MAX).is_none());
    }
}

/// 这才是 NewMessage
#[derive(Debug, Clone, Deserialize)]
pub struct NewMessage {
    #[serde(rename = "roomId")]
    pub room_id: RoomId,
    #[serde(rename = "message")]
    pub msg: Message,
}

impl NewMessage {
    /// 创建并初始化对应的数据结构。
    pub fn new(room_id: RoomId, msg: Message) -> Self { Self { room_id, msg } }

    /// 创建一条对这条消息的回复
    pub fn reply_with(&self, content: &str) -> SendMessage {
        SendMessage::new(content.to_string(), self.room_id, Some(self.msg.as_reply()))
    }

    /// 作为被删除的消息
    pub fn as_deleted(&self) -> DeleteMessage {
        DeleteMessage {
            room_id: self.room_id,
            message_id: self.msg.msg_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    /// 就是消息内容
    pub content: String,
    /// 发送的房间 id
    #[serde(rename = "roomId")]
    pub room_id: RoomId,
    /// 回复的消息
    #[serde(rename = "replyMessage")]
    pub reply_to: Option<ReplyMessage>,
    /// @ 谁
    #[serde(rename = "at")]
    pub at: JsonValue,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<ImageAttachment>,
    /// 是否当作表情发送
    ///
    /// 默认 false
    pub sticker: bool,
}

impl SendMessage {
    /// 创建并初始化对应的数据结构。
    pub fn new(content: String, room_id: RoomId, reply_to: Option<ReplyMessage>) -> Self {
        Self {
            content,
            room_id,
            reply_to,
            at: json!([]),
            media: Vec::new(),
            sticker: false,
        }
    }

    /// 返回当前值的 `value` 表示。
    pub fn as_value(&self) -> JsonValue { serde_json::to_value(self).unwrap() }

    pub fn has_base64_media(&self) -> bool {
        self.media
            .iter()
            .any(|attachment| attachment.b64.as_deref().is_some_and(|value| !value.is_empty()))
    }

    /// 设置消息的图片
    ///
    /// as_sticker: 是否当作表情发送
    /// file: 图片数据
    /// file_type: 图片类型(MIME) (image/png; image/jpeg)
    pub fn set_img(&mut self, file: &[u8], file_type: &str, as_sticker: bool) {
        self.sticker = as_sticker;
        use base64::{Engine as _, engine::general_purpose};
        let base64_data = general_purpose::STANDARD.encode(file);
        self.media.clear();
        self.add_img_data(format!("data:{file_type};base64,{base64_data}"), file_type);
    }

    pub fn add_img(&mut self, file: &[u8], file_type: &str) {
        use base64::{Engine as _, engine::general_purpose};
        let base64_data = general_purpose::STANDARD.encode(file);
        self.add_img_data(format!("data:{file_type};base64,{base64_data}"), file_type);
    }

    /// 设置语音媒体，编码/转码由 oicq Bridge 及其 FFmpeg 环境处理。
    /// 校验失败保留已有媒体；成功时替换媒体并关闭贴纸标记。
    pub fn set_audio(&mut self, file: &[u8], file_type: &str) -> Result<(), String> {
        if file.is_empty() || !valid_audio_type(file_type) {
            return Err("语音不能为空，MIME 必须为 audio/*".into());
        }
        use base64::{Engine as _, engine::general_purpose};
        let base64_data = general_purpose::STANDARD.encode(file);
        self.media.clear();
        self.sticker = false;
        self.add_img_data(format!("data:{file_type};base64,{base64_data}"), file_type);
        Ok(())
    }

    /// 复用协议侧语音资源，避免再次上传或重新编码。
    pub fn set_audio_fid(&mut self, fid: &str, file_type: &str) -> Result<(), String> {
        if fid.trim().is_empty() || !valid_audio_type(file_type) {
            return Err("语音 fid 不能为空，MIME 必须为 audio/*".into());
        }
        self.media = vec![ImageAttachment {
            b64: None,
            url: None,
            file_type: Some(file_type.into()),
            fid: Some(fid.into()),
            order: None,
        }];
        self.sticker = false;
        Ok(())
    }

    fn add_img_data(&mut self, b64: String, file_type: &str) {
        self.media.push(ImageAttachment {
            b64: Some(b64),
            url: None,
            file_type: Some(file_type.to_string()),
            fid: None,
            order: None,
        });
    }
}

fn valid_audio_type(file_type: &str) -> bool {
    file_type.strip_prefix("audio/").is_some_and(|subtype| {
        !subtype.is_empty()
            && subtype
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
    })
}

#[cfg(test)]
mod audio_tests {
    use super::SendMessage;

    #[test]
    fn audio_replaces_sticker_and_selects_http_instead_of_file_upload() {
        let mut message = SendMessage::new(String::new(), -30, None);
        message.set_img(b"test-image", "image/png", true);
        message.set_audio(b"test-audio", "audio/wav").unwrap();
        let value = message.as_value();
        assert!(message.has_base64_media());
        assert!(!message.sticker);
        assert_eq!(message.media.len(), 1);
        assert_eq!(value["media"][0]["type"], "audio/wav");
        assert!(value["media"][0]["b64"].as_str().unwrap().starts_with("data:audio/wav;base64,"));
        assert!(value.get("file").is_none());
    }

    #[test]
    fn invalid_audio_keeps_media_and_fid_reuse_avoids_http_upload() {
        let mut message = SendMessage::new(String::new(), 30, None);
        message.set_img(b"test-image", "image/png", true);
        let original = message.as_value();
        for mime in ["image/png", "audio/", "audio/wav;base64,", "audio/wav\n"] {
            assert!(message.set_audio(b"test-audio", mime).is_err());
            assert_eq!(message.as_value(), original);
        }
        assert!(message.set_audio(b"", "audio/wav").is_err());
        assert!(message.set_audio_fid(" ", "audio/wav").is_err());
        message.set_audio_fid("test-fid", "audio/silk").unwrap();
        assert!(!message.has_base64_media());
        assert!(!message.sticker);
        assert_eq!(message.media.len(), 1);
        assert_eq!(message.as_value()["media"][0]["fid"], "test-fid");
        assert!(message.as_value()["media"][0].get("b64").is_none());
    }
}

/// 被删除的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMessage {
    pub room_id: RoomId,
    pub message_id: MessageId,
}

impl DeleteMessage {
    /// 创建并初始化对应的数据结构。
    pub fn new(room_id: RoomId, message_id: MessageId) -> Self {
        Self {
            room_id,
            message_id,
        }
    }

    /// 返回当前值的 `value` 表示。
    pub fn as_value(&self) -> JsonValue { serde_json::to_value(self).unwrap() }
}

// #[cfg(test)]
// mod test {
//     use serde_json::json;

//     use super::*;

//     #[test]
//     fn test_new_from_json() {
//         let value = json!({"message": {"_id":"idddddd","anonymousId":null,"anonymousflag":null,"bubble_id":0,"content":"test","date":"2024/02/18","files":[],"role":"admin","senderId":123456,"subid":1,"time":1708267062000_i64,"timestamp":"22:37:42","title":"索引管理员","username":"shenjack"},"roomId":-123456});
//         let new_message = Message::new_from_json(&value);
//         assert_eq!(new_message.msg_id, "idddddd");
//         assert_eq!(new_message.sender_id, 123456);
//         assert_eq!(new_message.sender_name, "shenjack");
//         assert_eq!(new_message.content, "test");
//         assert_eq!(new_message.role, "admin");
//         assert_eq!(
//             new_message.time,
//             NaiveDateTime::from_timestamp_micros(1708267062000_i64).unwrap()
//         );
//         assert!(new_message.files.is_empty());
//         assert!(new_message.get_reply().is_none());
//         assert!(!new_message.is_reply());
//         assert!(!new_message.deleted);
//         assert!(!new_message.system);
//         assert!(!new_message.reveal);
//         assert!(!new_message.flash);
//         assert_eq!(new_message.title, "索引管理员");
//         assert!(new_message.anonymous_id.is_none());
//         assert!(!new_message.hide);
//         assert_eq!(new_message.bubble_id, 0);
//         assert_eq!(new_message.subid, 1);
//         assert!(new_message.head_img.is_null());
//     }

//     #[test]
//     fn test_parse_reply() {
//         let value = json!({"message": {"_id":"idddddd","anonymousId":null,"anonymousflag":null,"bubble_id":0,"content":"test","date":"2024/02/18","files":[],"role":"admin","senderId":123456,"subid":1,"time":1708267062000_i64,"timestamp":"22:37:42","title":"索引管理员","username":"shenjack", "replyMessage": {"content": "test", "username": "jackyuanjie", "files": [], "_id": "adwadaw"}},"roomId":-123456});
//         let new_message = Message::new_from_json(&value);
//         assert_eq!(new_message.get_reply().unwrap().sender_name, "jackyuanjie");
//         assert_eq!(new_message.get_reply().unwrap().content, "test");
//         assert_eq!(new_message.get_reply().unwrap().msg_id, "adwadaw");
//         assert!(new_message
//             .get_reply()
//             .unwrap()
//             .files
//             .as_array()
//             .unwrap()
//             .is_empty());
//     }
// }
