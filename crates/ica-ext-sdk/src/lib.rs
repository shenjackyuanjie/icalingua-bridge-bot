//! ica-bot 外部进程插件 SDK。stdout 只用于协议，插件日志请写入 stderr。

use std::{error::Error, fmt, io, str};

pub mod framing;
pub mod protocol;
mod runtime;

pub use async_trait::async_trait;
pub use framing::{FrameReader, MAX_LINE_BYTES, encode_frame, read_frame, write_frame};
pub use protocol::*;
pub use runtime::{HostClient, PluginHandler, PluginManifest, run};

/// SDK 或动作错误。宿主拒绝与连接中断有不同的动作完成语义。
#[derive(Debug)]
pub enum SdkError {
    Io(io::Error),
    Json(serde_json::Error),
    Utf8(str::Utf8Error),
    LineTooLong,
    TruncatedFrame,
    ExpectedHelloAck,
    /// 不再能取得回执，不能据此推断宿主没有执行动作。
    Closed,
    /// 宿主明确返回失败回执。
    Rejected(String),
    /// 收到成功回执但缺少该动作必需的结果数据。
    InvalidResponse(&'static str),
    RequestIdsExhausted,
    /// 插件回调异常退出。
    Callback(tokio::task::JoinError),
}

impl fmt::Display for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "插件管道 I/O 错误：{err}"),
            Self::Json(err) => write!(f, "插件协议 JSON 错误：{err}"),
            Self::Utf8(err) => write!(f, "插件协议 UTF-8 错误：{err}"),
            Self::LineTooLong => f.write_str("NDJSON 正文超过 16 MiB"),
            Self::TruncatedFrame => f.write_str("NDJSON 尾帧缺少换行"),
            Self::ExpectedHelloAck => f.write_str("宿主首帧必须为 hello_ack"),
            Self::Closed => f.write_str("插件动作通道已关闭，无法取得回执"),
            Self::Rejected(message) => write!(f, "宿主拒绝动作：{message}"),
            Self::InvalidResponse(action) => write!(f, "宿主的 {action} 回执缺少结果"),
            Self::RequestIdsExhausted => f.write_str("插件动作编号已耗尽"),
            Self::Callback(err) => write!(f, "插件回调异常退出：{err}"),
        }
    }
}

impl Error for SdkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Json(err) => Some(err),
            Self::Utf8(err) => Some(err),
            Self::Callback(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for SdkError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<str::Utf8Error> for SdkError {
    fn from(err: str::Utf8Error) -> Self {
        Self::Utf8(err)
    }
}
