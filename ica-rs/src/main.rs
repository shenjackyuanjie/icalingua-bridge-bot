use std::time::Duration;

mod config;
mod data_struct;
#[cfg(feature = "ica")]
mod ica;
#[cfg(feature = "matrix")]
mod matrix;
mod py;

use config::BotConfig;
use tracing::info;

#[allow(non_upper_case_globals)]
pub static mut ClientStatus_Global: ica::client::BotStatus = ica::client::BotStatus {
    login: false,
    current_loaded_messages_count: 0,
    online_data: None,
    rooms: None,
    config: None,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[macro_export]
macro_rules! wrap_callback {
    ($f:expr) => {
        |payload: Payload, client: Client| $f(payload, client).boxed()
    };
}

#[macro_export]
macro_rules! wrap_any_callback {
    ($f:expr) => {
        |event: Event, payload: Payload, client: Client| $f(event, payload, client).boxed()
    };
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
    info!("ica-async-rs v{}", VERSION);

    // 从命令行获取 host 和 key
    // 从命令行获取配置文件路径
    let bot_config = BotConfig::new_from_cli();
    ica::client::BotStatus::update_config(bot_config.clone());
    py::init_py(&bot_config);

    // 准备一个用于停止 socket 的变量
    let (send, recv) = tokio::sync::oneshot::channel::<()>();

    if bot_config.check_ica() {
        info!("启动 ica");
        let config = bot_config.ica();
        tokio::spawn(async move {
            ica::start_ica(&config, recv).await;
        });
    } else {
        info!("未启用 ica");
    }

    tokio::time::sleep(Duration::from_secs(2)).await;
    // 等待一个输入
    info!("Press any key to exit");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();

    send.send(()).ok();

    info!("Disconnected");
}
