use std::time::Duration;

mod config;
mod data_struct;
mod error;
mod py;
mod status;

#[cfg(feature = "ica")]
mod ica;
#[cfg(feature = "tailchat")]
mod tailchat;

use config::BotConfig;
use tracing::{event, info, span, Level};

pub static mut MAIN_STATUS: status::BotStatus = status::BotStatus {
    config: None,
    ica_status: None,
    tailchat_status: None,
};

pub type MainStatus = status::BotStatus;

pub type StopGetter = tokio::sync::oneshot::Receiver<()>;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ICA_VERSION: &str = "1.6.0";
pub const TAILCHAT_VERSION: &str = "1.1.0";

/// usage:
/// #[derive(Clone)]
/// struct BotState(String);
///
/// async fn some_event_with_state(payload: Payload, client: Client, state: Arc<BotState>) {
///    // do something
///
// macro_rules! wrap_callback_with_state {
//     ($f:expr, $state:tt) => {};
// }

#[tokio::main]
async fn main() {
    // -d -> debug
    // none -> info
    let level = {
        let args = std::env::args();
        let args = args.collect::<Vec<String>>();
        if args.contains(&"-d".to_string()) {
            Level::DEBUG
        } else if args.contains(&"-t".to_string()) {
            Level::TRACE
        } else {
            Level::INFO
        }
    };

    tracing_subscriber::fmt().with_max_level(level).init();
    let span = span!(Level::INFO, "Shenbot Main");
    let _enter = span.enter();

    event!(Level::INFO, "shenbot-async-rs v{} starting", VERSION);

    let bot_config = BotConfig::new_from_cli();
    MainStatus::static_init(bot_config);
    let bot_config = MainStatus::global_config();

    py::init_py();

    // 准备一个用于停止 socket 的变量
    event!(Level::INFO, "启动 ICA");
    let (ica_send, ica_recv) = tokio::sync::oneshot::channel::<()>();

    if bot_config.check_ica() {
        event!(Level::INFO, "启动 ica");
        let config = bot_config.ica();
        tokio::spawn(async move {
            ica::start_ica(&config, ica_recv).await.unwrap();
        });
    } else {
        event!(Level::INFO, "未启用 ica");
    }

    let (tailchat_send, tailchat_recv) = tokio::sync::oneshot::channel::<()>();

    if bot_config.check_tailchat() {
        event!(Level::INFO, "启动 Tailchat");
        let config = bot_config.tailchat();
        tokio::spawn(async move {
            tailchat::start_tailchat(config, tailchat_recv).await.unwrap();
        });
    } else {
        event!(Level::INFO, "未启用 Tailchat");
    }

    tokio::time::sleep(Duration::from_secs(2)).await;
    // 等待一个输入
    info!("Press any key to exit");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();

    ica_send.send(()).ok();
    tailchat_send.send(()).ok();

    info!("Disconnected");
}
