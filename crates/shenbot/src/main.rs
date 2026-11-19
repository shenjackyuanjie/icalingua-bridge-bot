//! 命令行、日志和 Tokio 运行时入口。

use shenbot::{ForcedShutdown, VERSION, init_start_up_time, run, version_str};
use tracing::{Level, event};

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

/// 解析命令行和配置、初始化日志，并启动异步运行时。
fn main() -> anyhow::Result<()> {
    init_start_up_time();

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

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(true)
        .with_ansi_sanitization(false)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("shenbot-rs")
        .worker_threads(10)
        .build()
        .unwrap();

    let result = rt.block_on(run());

    event!(Level::INFO, "shenbot-rs v{} exiting", VERSION);

    match result {
        Ok(()) => Ok(()),
        Err(e) if e.is::<ForcedShutdown>() => {
            // 不能等待运行时或不可取消的 spawn_blocking 任务 Drop。
            event!(Level::WARN, "{e}");
            std::process::exit(130);
        }
        Err(e) => {
            event!(Level::ERROR, "shenbot-rs v{} exiting with error: {}", VERSION, e);
            Err(e)
        }
    }
}
