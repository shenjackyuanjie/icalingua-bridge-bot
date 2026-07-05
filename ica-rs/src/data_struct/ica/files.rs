//! Icalingua 消息文件附件数据结构。

use serde::{Deserialize, Serialize};

/*interface MessageFile {
    type: string
    url: string
    size?: number
    name?: string
    fid?: string
}
 */
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub url: String,
    pub size: Option<i32>,
    pub name: Option<String>,
    pub fid: Option<String>,
}

impl MessageFile {
    /// 返回 `name` 对应的数据。
    pub fn get_name(&self) -> Option<&String> { self.name.as_ref() }
    /// 返回 `fid` 对应的数据。
    pub fn get_fid(&self) -> Option<&String> { self.fid.as_ref() }
}
