//! Shenbot 宿主和聊天后端。

use std::time::SystemTime;

mod commands;
/// 加载 `config` 子模块。
pub mod config;
pub mod context;
pub mod host;
pub use host::runtime::{ForcedShutdown, run};
/// 加载 `data_struct` 子模块。
pub mod data_struct;
/// 加载 `error` 子模块。
pub mod error;
pub mod ext;
/// 加载 `py` 子模块。
mod py;
/// 加载 `status` 子模块。
pub mod status;

#[cfg(feature = "ica")]
/// 加载 `ica` 子模块。
mod ica;
#[cfg(feature = "tailchat")]
/// 加载 `tailchat` 子模块。
mod tailchat;

pub type StopGetter = tokio::sync::oneshot::Receiver<()>;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ICA_VERSION: &str = "2.0.6";
pub const TAILCHAT_VERSION: &str = "2.0.0";

/// 获取帮助信息。
pub fn help_msg() -> String { commands::help(&client_id()) }

/// CLI 解析前记录进程启动时间，交给其创建的运行实例。
pub fn init_start_up_time() { let _ = context::process_startup(); }

/// 兼容读取 CLI 进程的启动时间；运行实例应读 BotContext。
pub fn start_up_time() -> SystemTime { context::process_startup().time() }

/// 兼容读取 CLI 进程的六位客户端 ID；运行实例应读 BotContext。
pub fn client_id() -> String { context::process_startup().client_id().to_owned() }

/// 兼容读取 CLI 版本信息。
pub fn version_str() -> String { version_for(context::process_startup().client_id()) }

pub(crate) fn version_for(client_id: &str) -> String {
    format!(
        "shenbot-rs v{}{}-[{}] ica v{}({}) tailchat v{}",
        VERSION,
        if STABLE { "" } else { "-开发版" },
        client_id,
        ICA_VERSION,
        ica::ICA_PROTOCOL_VERSION,
        TAILCHAT_VERSION,
    )
}

/// 是否为稳定版本
/// 会在 release 的时候设置为 true
pub const STABLE: bool = false;

#[macro_export]
macro_rules! async_callback_with_state {
    ($f:expr, $state:expr) => {{
        use futures_util::FutureExt;
        let state = $state.clone();
        move |payload: Payload, client: Client| $f(payload, client, state.clone()).boxed()
    }};
}

#[macro_export]
macro_rules! async_any_callback_with_state {
    ($f:expr, $state:expr) => {{
        use futures_util::FutureExt;
        let state = $state.clone();
        move |event: Event, payload: Payload, client: Client| {
            $f(event, payload, client, state.clone()).boxed()
        }
    }};
}
