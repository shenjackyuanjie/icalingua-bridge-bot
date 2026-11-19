//! Icalingua 原始消息节点的构造和序列化接口。

use serde_json::Value as JsonValue;

/// 加载 `node_types` 子模块。
pub mod node_types;

pub use node_types::MusicPlatform;

use crate::data_struct::ica::{MessageId, RoomId};

/// 原始消息节点
///
/// 只包括了最常用的几种消息节点
///
/// 可能会有更多类型的节点
///
/// 所以带上了 non_exhaustive
#[non_exhaustive]
pub enum MsgNode {
    /// 文字消息
    Text(String),
    /// at人
    At,
    /// 经典表情
    Face,
    /// 小黄脸表情
    SFace,
    /// 原创表情
    Bface,
    /// 猜拳
    Rps(u8),
    /// 骰子
    Dice(u8),
    /// 音乐
    Music {
        /// 音乐平台
        platform: MusicPlatform,
        /// 音乐 ID
        id: String,
    },
    /// 链接分享
    Share {
        url: String,
        title: String,
        content: Option<String>,
        image: Option<String>,
    },
    /// json 消息
    Json {
        data: JsonValue,
        text: Option<String>,
    },
    /// xml 消息
    Xml {
        data: String,
        r#type: Option<i64>,
        text: Option<String>,
    },
    /// 匿名消息
    Anonymous {
        /// 是否在无法匿名时以普通形式继续发送
        ignore: Option<bool>,
    },
    /// 回复消息
    ///
    /// 温馨提示: 一般来说这玩意要丢在最前面
    /// 但是显然你也可以把他放在中间或者随便什么地方
    Reply { id: MessageId, text: Option<String> },
    /// node?
    Node { id: MessageId },
    /// 窗口抖动
    Shake,
    /// 戳一戳
    Poke { r#type: i32, id: i32 },
    /// mirai 系用于标记东西的玩意
    ///
    /// 发送后会回传, 可以用于标记消息
    Mirai {
        /// TODO: 添加一些自定义标记
        data: String,
    },
    /// markdown 信息
    Markdown {
        markdown: String,
        unknown: Option<i32>,
        time: Option<i32>,
        token: Option<String>,
    },
}

impl MsgNode {
    /// 返回配置值类型。
    pub fn type_of(&self) -> &str {
        match self {
            MsgNode::Text(_) => "text",
            MsgNode::At => "at",
            MsgNode::Face => "face",
            MsgNode::SFace => "sface",
            MsgNode::Bface => "bface",
            MsgNode::Dice(_) => "dice",
            MsgNode::Rps(_) => "rps",
            MsgNode::Music { .. } => "music",
            MsgNode::Share { .. } => "share",
            MsgNode::Json { .. } => "json",
            MsgNode::Xml { .. } => "xml",
            _ => "unknown",
        }
    }
}

#[derive(Default)]
pub struct RawSendMessage {
    pub msg_nodes: Vec<MsgNode>,
    pub room_id: RoomId,
    // pub message_type: String,
    // 直接就是 raw
    // at: AtCacheItem[]
    // 到时候输出的时候直接加一个 [] 即可
    // content: string
    // content 就是具体内容
}

impl RawSendMessage {
    /// 创建尚未设置房间和节点的原始消息构造器。
    pub fn new() -> Self { Self::default() }

    /// Python 实际入口：验证原始节点链，再按 SendMessageParams 构造 wire 参数。
    pub fn string_to_json(data: &str, room: RoomId) -> Result<JsonValue, String> {
        let nodes: JsonValue = serde_json::from_str(data)
            .map_err(|error| format!("raw 消息不是有效 JSON：{error}"))?;
        validate_node_chain(&nodes)?;
        Ok(serde_json::json!({
            "messageType": "raw",
            "roomId": room,
            "content": nodes.to_string(),
            "at": [],
        }))
    }
}

/// Bridge 对 JSON.parse(content) 的结果执行 spread，因此只能接收节点数组。
/// 仅校验共同结构，不收窄 oicq 的扁平/嵌套 data 节点或扩展类型。
pub fn validate_node_chain(value: &JsonValue) -> Result<(), String> {
    let nodes = value
        .as_array()
        .filter(|nodes| !nodes.is_empty())
        .ok_or_else(|| "raw 消息必须是非空节点数组".to_string())?;
    for (index, node) in nodes.iter().enumerate() {
        let node = node
            .as_object()
            .ok_or_else(|| format!("raw 消息第 {} 个节点必须是对象", index + 1))?;
        if !node
            .get("type")
            .and_then(JsonValue::as_str)
            .is_some_and(|kind| !kind.trim().is_empty())
        {
            return Err(format!("raw 消息第 {} 个节点必须具有非空字符串 type", index + 1));
        }
    }
    Ok(())
}
