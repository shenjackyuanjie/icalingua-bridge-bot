//! Python 业务调用的参数与消息归属规则，基于本地 Socket.IO + oicq Bridge。

use std::time::Duration;

use pyo3::{Bound, PyAny, PyResult, exceptions::PyValueError};
use serde_json::{Value, json};

use crate::data_struct::ica::{
    RoomId,
    messages::{Message, NewMessage},
};
use crate::py::json::py_to_json;

#[cfg(test)]
#[path = "tests/bridge_contract.rs"]
mod tests;

pub fn query_timeout(seconds: f64) -> PyResult<Duration> {
    if !seconds.is_finite() || seconds <= 0.0 || seconds > 300.0 {
        return Err(PyValueError::new_err("timeout 必须在 (0, 300] 秒范围内"));
    }
    Ok(Duration::from_secs_f64(seconds))
}

pub fn json_args(args: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Value>> {
    let value = args.map(py_to_json).transpose()?.unwrap_or_else(|| json!([]));
    value
        .as_array()
        .cloned()
        .ok_or_else(|| PyValueError::new_err("args/at 必须是 JSON 参数数组"))
}

pub fn decode_query_messages(
    values: Vec<Value>,
    fallback_room: RoomId,
    require_source: bool,
) -> Result<Vec<NewMessage>, String> {
    values
        .into_iter()
        .map(|value| {
            let message = Message::new_from_json(&value)
                .map_err(|_| "Bridge 查询返回了无效 Message，未写入缓存".to_string())?;
            let room = message.room_id.unwrap_or(fallback_room);
            // 跨房间查询必须有来源；转发资源的独立 room=0 由调用者明确不写缓存。
            if require_source && room == 0 {
                return Err("跨房间查询缺少有效 roomId，未写入缓存".into());
            }
            if fallback_room != 0 && room != fallback_room {
                return Err("房间查询返回了不同 roomId，未写入缓存".into());
            }
            Ok(NewMessage::new(room, message))
        })
        .collect()
}

pub fn validate_mentions(value: &Value) -> PyResult<()> {
    let mentions = value.as_array().ok_or_else(|| PyValueError::new_err("发送 at 必须是数组"))?;
    for mention in mentions {
        let valid_id = mention
            .get("id")
            .is_some_and(|id| id.as_i64().is_some_and(|id| id > 0) || id.as_str() == Some("all"));
        let valid_text = mention.get("text").is_none_or(Value::is_string);
        if !mention.is_object() || !valid_id || !valid_text {
            return Err(PyValueError::new_err(
                "at 项必须包含正数 id 或 'all'，text 若提供则为字符串",
            ));
        }
    }
    Ok(())
}

pub fn validate_forward_nodes(nodes: &Value, target: RoomId) -> PyResult<()> {
    if target == 0 {
        return Err(PyValueError::new_err("合并转发目标不能为 0"));
    }
    let nodes = nodes
        .as_array()
        .filter(|nodes| !nodes.is_empty())
        .ok_or_else(|| PyValueError::new_err("合并转发 nodes 必须是非空 FakeMessage 数组"))?;
    for node in nodes {
        let sender = node.get("user_id").and_then(Value::as_i64).is_some_and(|id| id > 0);
        let message = node.get("message").is_some_and(|message| {
            message.as_str().is_some_and(|text| !text.is_empty())
                || message.as_array().is_some_and(|elements| {
                    !elements.is_empty()
                        && elements.iter().all(|element| {
                            element
                                .get("type")
                                .and_then(Value::as_str)
                                .is_some_and(|kind| !kind.is_empty())
                        })
                })
        });
        if !sender || !message {
            return Err(PyValueError::new_err(
                "FakeMessage 需要正数 user_id 与非空 message 节点链/文本",
            ));
        }
        if node.get("time").is_some_and(|time| !time.is_i64()) {
            return Err(PyValueError::new_err("FakeMessage.time 使用秒级整数时间"));
        }
    }
    Ok(())
}

/// 规范化消息没有 raw 链时显式降级；不把普通文件 URL 冒充为可转发的 QQ 文件资源。
pub fn forward_node(message: &Message) -> Value {
    let elements = if let Some(nodes) = message.raw_msg.as_array().filter(|nodes| !nodes.is_empty())
    {
        nodes.clone()
    } else {
        let mut elements = Vec::new();
        if let Some(reply) = &message.reply {
            elements.push(json!({"type":"reply","data":{"id":reply.msg_id,"text":reply.content}}));
        }
        if let Some(code) = message.code.as_str().filter(|code| !code.trim().is_empty()) {
            let kind = if code.trim_start().starts_with('<') {
                "xml"
            } else {
                "json"
            };
            elements.push(json!({"type":kind,"data":{"data":code}}));
        } else if !message.content.is_empty() {
            elements.push(json!({"type":"text","data":{"text":message.content}}));
        }
        for file in &message.files {
            if file.file_type == "image" || file.file_type.starts_with("image/") {
                if !file.url.is_empty() {
                    elements.push(json!({"type":"image","data":{"file":file.url,"type":"image"}}));
                }
            } else if file.file_type.starts_with("audio/") {
                if let Some(fid) = file.fid.as_deref().filter(|fid| !fid.is_empty()) {
                    elements.push(json!({"type":"record","data":{"file":fid}}));
                }
                elements.push(json!({"type":"text","data":{"text":"[语音可能无法合并转发]"}}));
            } else {
                let name = file.name.as_deref().unwrap_or("附件");
                elements.push(
                    json!({"type":"text","data":{"text":format!("[{name} 不支持合并转发]")}}),
                );
            }
        }
        if elements.is_empty() {
            elements.push(json!({"type":"text","data":{"text":"[空消息]"}}));
        }
        elements
    };
    json!({
        "user_id":message.sender_id,
        "nickname":message.sender_name,
        "time":message.time.timestamp(),
        "id":message.msg_id,
        "message":elements,
        "consistent":true,
        "bubble_id":message.bubble_id,
    })
}
