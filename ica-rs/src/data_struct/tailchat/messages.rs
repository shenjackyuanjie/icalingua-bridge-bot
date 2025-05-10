use std::fmt::Display;

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::data_struct::tailchat::{ConverseId, GroupId, MessageId, UserId};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReceiveMessage {
    /// 消息ID
    #[serde(rename = "_id")]
    pub msg_id: MessageId,
    /// 消息内容
    pub content: String,
    /// 发送者ID
    #[serde(rename = "author")]
    pub sender_id: UserId,
    /// 服务器ID
    /// 在私聊中不存在
    #[serde(rename = "groupId")]
    pub group_id: Option<GroupId>,
    /// 会话ID
    #[serde(rename = "converseId")]
    pub converse_id: ConverseId,
    /// 是否有回复?
    #[serde(rename = "hasRecall")]
    pub has_recall: bool,
    /// 暂时懒得解析这玩意
    /// 准确来说是不确定内容, 毕竟没细看 API
    pub meta: Option<JsonValue>,
    /// 也懒得解析这玩意
    pub reactions: Vec<JsonValue>,
    /// 创建时间
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// 更新时间
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

impl ReceiveMessage {
    pub fn is_reply(&self) -> bool {
        if let Some(meta) = &self.meta {
            meta.get("reply").is_some()
        } else {
            false
        }
    }

    pub fn is_from_self(&self) -> bool {
        crate::MainStatus::global_tailchat_status().user_id == self.sender_id
    }

    /// 创建一个对这条消息的回复
    pub fn as_reply(&self) -> SendingMessage {
        SendingMessage::new(
            "".to_string(),
            self.converse_id.clone(),
            self.group_id.clone(),
            Some(ReplyMeta::from_receive_message(self)),
        )
    }

    /// 回复这条消息
    pub fn reply_with(&self, content: &str) -> SendingMessage {
        SendingMessage::new(
            content.to_string(),
            self.converse_id.clone(),
            self.group_id.clone(),
            Some(ReplyMeta::from_receive_message(self)),
        )
    }
}

impl Display for ReceiveMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // msgid|groupid-converseid|senderid|content
        write!(
            f,
            "{}|{:?}-{}|{}|{}",
            self.msg_id, self.group_id, self.converse_id, self.sender_id, self.content
        )
    }
}

#[derive(Debug, Clone, Default)]
pub enum SendingFile {
    #[default]
    None,
    /// 需要生成
    /// [img height=1329 width=1918]{BACKEND}/static/files/6602e20d7b8d10675758e36b/8db505b87bdf9fb309467abcec4d8e2a.png[/img]
    Image { file: Vec<u8>, name: String },
    /// [card type=file url={BACKEND}/static/files/6602e20d7b8d10675758e36b/9df28943d17b9713cb0ea9625f37d015.wav]Engine.wav[/card]
    File { file: Vec<u8>, name: String },
}

impl SendingFile {
    pub fn is_some(&self) -> bool { !matches!(self, Self::None) }
    pub fn is_image(&self) -> bool { matches!(self, Self::Image { .. }) }
    pub fn is_file(&self) -> bool { matches!(self, Self::File { .. }) }

    pub fn file_data(&self) -> Vec<u8> {
        match self {
            Self::Image { file, .. } => file.clone(),
            Self::File { file, .. } => file.clone(),
            _ => vec![],
        }
    }

    pub fn file_name(&self) -> String {
        match self {
            Self::Image { name, .. } => name.clone(),
            Self::File { name, .. } => name.clone(),
            _ => "".to_string(),
        }
    }
    pub fn gen_markdown(&self, backend_path: &str) -> String {
        match self {
            Self::Image { .. } => {
                format!("[img]{backend_path}[/img]")
            }
            Self::File { name, .. } => {
                format!("[card type=file url={backend_path}]{name}[/card]")
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
/// 将要发送的消息
///
/// 发送时:
/// - `content`: 回复的消息内容
/// - `converseId`: 会话ID
/// - `groupId`: 服务器ID
/// - `meta`: 回复的消息的元数据 （ 可能为空 ）
///   - `mentions`: 被回复的人的ID (可以是多个)
///   - `reply`: 被回复的消息
///     - `_id`: 被回复的消息ID
///     - `author`: 被回复的消息的发送者ID
///     - `content`: 被回复的消息内容
pub struct SendingMessage {
    /// 消息内容
    ///
    /// 其实还有个 plain, 就是不知道干啥的
    pub content: String,
    /// 会话ID
    #[serde(rename = "converseId")]
    pub converse_id: ConverseId,
    /// 服务器ID
    #[serde(rename = "groupId")]
    pub group_id: Option<GroupId>,
    /// 消息的元数据
    pub meta: Option<ReplyMeta>,
    /// 额外携带的文件
    #[serde(skip)]
    pub file: SendingFile,
}

impl SendingMessage {
    pub fn new(
        content: String,
        converse_id: ConverseId,
        group_id: Option<GroupId>,
        meta: Option<ReplyMeta>,
    ) -> Self {
        Self {
            content,
            converse_id,
            group_id,
            meta,
            file: SendingFile::None,
        }
    }
    pub fn new_without_meta(
        content: String,
        converse_id: ConverseId,
        group_id: Option<GroupId>,
    ) -> Self {
        Self {
            content,
            converse_id,
            group_id,
            meta: None,
            file: SendingFile::None,
        }
    }
    pub fn contain_file(&self) -> bool { self.file.is_some() }

    pub fn add_img(&mut self, file: SendingFile) { self.file = file; }

    pub fn as_value(&self) -> JsonValue { serde_json::to_value(self).unwrap() }
}

#[derive(Debug, Clone)]
pub struct ReplyMeta {
    /// 被回复的人的ID (可以是多个)
    pub mentions: Vec<UserId>,
    /// 被回复的消息ID
    pub reply_id: MessageId,
    /// 被回复的消息的发送者ID
    pub reply_author: UserId,
    /// 被回复的消息内容
    pub reply_content: String,
}

impl ReplyMeta {
    pub fn from_receive_message(msg: &ReceiveMessage) -> Self {
        Self {
            mentions: vec![msg.sender_id.clone()],
            reply_id: msg.msg_id.clone(),
            reply_author: msg.sender_id.clone(),
            reply_content: msg.content.clone(),
        }
    }
    pub fn add_mention(&mut self, user_id: UserId) { self.mentions.push(user_id); }
    pub fn replace_content(&mut self, content: String) { self.reply_content = content; }
}

impl Serialize for ReplyMeta {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let reply = json! {
            {
                "_id": self.reply_id,
                "author": self.reply_author,
                "content": self.reply_content,
            }
        };
        let mut map = serde_json::Map::new();
        map.insert("mentions".to_string(), serde_json::to_value(&self.mentions).unwrap());
        map.insert("reply".to_string(), reply);
        map.serialize(serializer)
    }
}
