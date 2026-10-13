//! Icalingua 消息文件附件数据结构。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// oicq Bridge 的规范化附件；位置采用 UTF-16 偏移，大小采用非负字节数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u64>,
    #[serde(
        rename = "isFace",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub is_face: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    /// 保留 oicq 扩展字段，避免重发/事件更新时丢掉媒体元数据。
    #[serde(flatten)]
    pub extra: BTreeMap<String, JsonValue>,
}

impl MessageFile {
    /// 返回 `name` 对应的数据。
    pub fn get_name(&self) -> Option<&String> { self.name.as_ref() }
    /// 返回 `fid` 对应的数据。
    pub fn get_fid(&self) -> Option<&String> { self.fid.as_ref() }
}
