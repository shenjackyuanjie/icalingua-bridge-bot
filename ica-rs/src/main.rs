use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::OnceLock,
    time::{Duration, SystemTime},
};

mod config;
mod data_struct;
mod error;
mod py;
mod status;
mod wasms;

#[cfg(feature = "ica")]
mod ica;
#[cfg(feature = "tailchat")]
mod tailchat;

use colored::Colorize;
use config::BotConfig;
use error::PyPluginError;
use tracing::{Level, event, span};

pub static mut MAIN_STATUS: status::BotStatus = status::BotStatus {
    config: None,
    ica_status: None,
    tailchat_status: None,
};

pub type MainStatus = status::BotStatus;

pub type StopGetter = tokio::sync::oneshot::Receiver<()>;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ICA_VERSION: &str = "2.0.1";
pub const TAILCHAT_VERSION: &str = "2.0.0";

const HELP_MSG: &str = r#"/bot-rs
    展示 rust 侧信息
/bot-py
    展示 python 侧信息(如果python插件启用了的话)
/bot-ls
    显示所有插件信息
/bot-permission
    显示自己的权限组
/bot-enable-<client-id> <plugin>
    启用某个插件(具体到客户端)
/bot-disable-<client-id> <plugin>
    禁用某个插件(具体到客户端)
/bot-reload-<client-id> <plugin>
    重载某个插件(具体到客户端)

by shenjackyuanjie"#;

/// 获取帮助信息
pub fn help_msg() -> String {
    format!("{}\n{}", version_str(), HELP_MSG).replace("<client-id>", client_id().as_str())
}

static STARTUP_TIME: OnceLock<SystemTime> = OnceLock::new();

pub fn start_up_time() -> SystemTime { *STARTUP_TIME.get().expect("WTF, why did you panic?") }

/// 获得当前客户端的 id
/// 防止串号
pub fn client_id() -> String {
    let mut hasher = DefaultHasher::new();
    start_up_time().hash(&mut hasher);
    let data = hasher.finish();
    // 取后6位
    format!("{:06}", data % 1_000_000)
}

/// 获取版本信息
pub fn version_str() -> String {
    format!(
        "shenbot-rs v{}{}-[{}] ica v{}({}) tailchat v{}",
        VERSION,
        if STABLE { "" } else { "-开发版" },
        client_id(),
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

const CLI_HELP_MSG: &str = r#"{VERSION}
    -d
        debug 模式
    -t
        trace 模式
    -h
        显示帮助信息
    -env <env>
        指定虚拟环境路径
    -c <config_file_path>
        指定配置文件路径
"#;

fn main() -> anyhow::Result<()> {
    let start_up_time = SystemTime::now();
    STARTUP_TIME.set(start_up_time).expect("WTF, why did you panic?");

    // -d -> debug
    // none -> info
    let args = std::env::args();
    let args = args.collect::<Vec<String>>();
    if args.contains(&"-h".to_string()) {
        println!("{}", CLI_HELP_MSG.replace("{VERSION}", version_str().as_str()));
        return Ok(());
    }
    let level = {
        if args.contains(&"-d".to_string()) {
            Level::DEBUG
        } else if args.contains(&"-t".to_string()) {
            Level::TRACE
        } else {
            Level::INFO
        }
    };

    tracing_subscriber::fmt().with_max_level(level).init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("shenbot-rs")
        .worker_threads(10)
        .build()
        .unwrap();

    let result = rt.block_on(inner_main());

    event!(Level::INFO, "shenbot-rs v{} exiting", VERSION);

    match result {
        Ok(_) => {}
        Err(e) => {
            if let Some(PyPluginError::PluginNotStopped) = e.downcast_ref::<PyPluginError>() {
                event!(Level::WARN, "Python 插件停不下来, 3s 后终止 tokio rt");
                rt.shutdown_timeout(Duration::from_secs(3));
            } else {
                event!(Level::ERROR, "shenbot-rs v{} exiting with error: {}", VERSION, e);
            }
        }
    }

    Ok(())
}

async fn inner_main() -> anyhow::Result<()> {
    let span = span!(Level::INFO, "bot-main");
    let _enter = span.enter();

    event!(Level::INFO, "shenbot-rs v{} starting", VERSION);
    if !STABLE {
        event!(Level::WARN, "这是一个开发版本, 有问题记得找 shenjack");
    }

    let bot_config = BotConfig::new_from_cli();
    MainStatus::static_init(bot_config);
    let bot_config = MainStatus::global_config();

    if bot_config.check_py() {
        py::init_py().await;
    }

    // 准备一个用于停止 socket 的变量
    let (ica_send, ica_recv) = tokio::sync::oneshot::channel::<()>();

    if bot_config.check_ica() {
        event!(Level::INFO, "{}", "开始启动 ICA".green());
        let config = bot_config.ica();
        tokio::spawn(async move {
            ica::start_ica(&config, ica_recv).await.unwrap();
        });
    } else {
        event!(Level::INFO, "{}", "ica 未启用, 不管他".cyan());
    }

    let (tailchat_send, tailchat_recv) = tokio::sync::oneshot::channel::<()>();

    if bot_config.check_tailchat() {
        event!(Level::INFO, "{}", "开始启动 tailchat".green());
        let config = bot_config.tailchat();
        tokio::spawn(async move {
            tailchat::start_tailchat(config, tailchat_recv).await.unwrap();
        });
    } else {
        event!(Level::INFO, "{}", "tailchat 未启用, 不管他".bright_magenta());
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    // 等待一个输入
    event!(Level::INFO, "Press ctrl+c to exit, second ctrl+c to force exit");
    tokio::signal::ctrl_c().await.ok();

    ica_send.send(()).ok();
    tailchat_send.send(()).ok();

    event!(Level::INFO, "Disconnected");

    py::post_py().await?;

    event!(Level::INFO, "Shenbot-rs exiting");

    Ok(())
}

#[allow(dead_code, unused_variables)]
#[cfg(test)]
#[tokio::test]
async fn test_macro() {
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use rust_socketio::Payload;
    use rust_socketio::asynchronous::{Client, ClientBuilder};

    /// 一个简单的例子
    #[derive(Clone)]
    struct BotState(String);

    /// 一个复杂一些的例子
    #[derive(Clone)]
    struct BotState2 {
        pub name: Arc<RwLock<String>>,
    }

    async fn some_event_with_state(payload: Payload, client: Client, state: Arc<BotState>) {
        // do something with your state
    }

    async fn some_state_change_event(payload: Payload, client: Client, state: Arc<BotState2>) {
        if let Payload::Text(text) = payload {
            if let Some(first_one) = text.first() {
                let new_name = first_one.as_str().unwrap_or_default();
                let old_name = state.name.read().await;
                if new_name != *old_name {
                    // update your name here
                    *state.name.write().await = new_name.to_string();
                }
            }
        }
    }

    let state = Arc::new(BotState("hello".to_string()));
    let state2 = Arc::new(BotState2 {
        name: Arc::new(RwLock::new("hello".to_string())),
    });
    let socket = ClientBuilder::new("http://example.com")
        .on("message", async_callback_with_state!(some_event_with_state, state))
        .on("update_status", async_callback_with_state!(some_state_change_event, state2))
        .connect()
        .await;
}
