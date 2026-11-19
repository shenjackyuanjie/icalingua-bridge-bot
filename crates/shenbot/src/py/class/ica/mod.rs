//! 暴露给 Python 插件的 Icalingua 消息、房间和客户端类型。

mod bridge_contract;
mod client;
mod message;
mod room;
mod send;

pub use client::{IcaClientPy, IcaStatusPy};
pub use message::{DeleteMessagePy, NewMessagePy, ReplyMessagePy};
pub use room::{IcaGroupMemberPy, IcaJoinRequestPy, IcaRoomPy};
pub use send::SendMessagePy;

// 保持拆分前的客户端日志过滤路径。
const LOG_TARGET: &str = module_path!();
