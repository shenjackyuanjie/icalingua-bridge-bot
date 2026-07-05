//! Tailchat HTTP API 请求数据结构。

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileUpload {
    pub etag: String,
    pub path: String,
    pub url: String,
}
