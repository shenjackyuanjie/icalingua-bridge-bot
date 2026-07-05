//! Tailchat API 使用的数据结构集合。

/// 加载 `api` 子模块。
pub mod api;
/// 加载 `messages` 子模块。
pub mod messages;
/// 加载 `status` 子模块。
pub mod status;

pub type GroupId = String;
pub type ConverseId = String;
pub type UserId = String;
pub type MessageId = String;
