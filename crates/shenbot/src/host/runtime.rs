//! 运行时监督：同一 Ctrl+C 订阅覆盖启动、插件排空和聊天后端断连。
use crate::{
    STABLE, VERSION,
    config::BotConfig,
    context::{BotContext, process_startup},
    host::BotHost,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::oneshot,
    task::{JoinError, JoinSet},
};

#[derive(Debug)]
pub struct ForcedShutdown;

impl std::fmt::Display for ForcedShutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("第二次 Ctrl+C，强制退出")
    }
}

impl std::error::Error for ForcedShutdown {}

#[cfg(windows)]
fn interrupts() -> std::io::Result<tokio::signal::windows::CtrlC> {
    tokio::signal::windows::ctrl_c()
}

#[cfg(unix)]
fn interrupts() -> std::io::Result<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
}

#[derive(Default)]
struct Backends {
    tasks: JoinSet<(&'static str, anyhow::Result<()>)>,
    stop: Vec<oneshot::Sender<()>>,
}

impl Backends {
    fn start(&mut self, host: &Arc<BotHost>) {
        if host.context.config.check_ica() {
            let host = host.clone();
            let (stop, receiver) = oneshot::channel();
            self.stop.push(stop);
            self.tasks.spawn(async move {
                let config = host.context.config.ica().clone();
                ("ICA", crate::ica::start_ica(&config, receiver, host).await.map_err(Into::into))
            });
        }
        if host.context.config.check_tailchat() {
            let host = host.clone();
            let (stop, receiver) = oneshot::channel();
            self.stop.push(stop);
            self.tasks.spawn(async move {
                let config = host.context.config.tailchat().clone();
                (
                    "Tailchat",
                    crate::tailchat::start_tailchat(config, receiver, host)
                        .await
                        .map_err(Into::into),
                )
            });
        }
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        for stop in self.stop.drain(..) {
            let _ = stop.send(());
        }
        let mut first_error = None;
        let disconnected = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(result) = self.tasks.join_next().await {
                if let Some(error) = backend_error(result, false) {
                    tracing::warn!(%error, "聊天后端退出异常");
                    first_error.get_or_insert(error);
                }
            }
        })
        .await;
        if disconnected.is_err() {
            self.tasks.abort_all();
            tracing::warn!("聊天后端未在 5 秒内断开，已要求中止监督任务");
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn backend_error(
    result: Result<(&'static str, anyhow::Result<()>), JoinError>,
    early: bool,
) -> Option<anyhow::Error> {
    match result {
        Ok((name, Ok(()))) if early => Some(anyhow::anyhow!("{name} 任务在停止信号前结束")),
        Ok((_, Ok(()))) => None,
        Ok((name, Err(error))) => Some(error.context(format!("{name} 后端失败"))),
        Err(error) => Some(anyhow::anyhow!("聊天后端任务异常终止: {error}")),
    }
}

/// 启动已启用的插件与聊天后端；可由二进制将强制退出映射为真正的进程退出。
pub async fn run() -> anyhow::Result<()> {
    let mut interrupts = interrupts()?;
    let startup = process_startup().clone();
    tracing::info!("shenbot-rs v{VERSION} starting");
    if !STABLE {
        tracing::warn!("这是一个开发版本, 有问题记得找 shenjack");
    }
    let context = BotContext::with_startup(
        BotConfig::new_from_cli(),
        tokio::runtime::Handle::current(),
        startup,
    )?;
    let host = BotHost::new(context)?;
    let mut backends = Backends::default();
    let starting = host.start();
    tokio::pin!(starting);

    let (startup_interrupted, first_ctrl_c, failure) = tokio::select! {
        biased;
        _ = interrupts.recv() => (true, true, None),
        result = &mut starting => {
            result?;
            backends.start(&host);
            tracing::info!("按 Ctrl+C 开始优雅退出，再按一次强制退出");
            tokio::select! {
                biased;
                _ = interrupts.recv() => (false, true, None),
                result = backends.tasks.join_next(), if !backends.tasks.is_empty() => {
                    (false, false, result.and_then(|result| backend_error(result, true)))
                }
            }
        }
    };

    host.stop_accepting();
    tracing::info!("停止接收新命令与插件事件，开始排空插件；此阶段保留聊天连接");
    let graceful = async {
        // 启动事务若已开始，先等它交回资源；第二次 Ctrl+C 仍然可以打断。
        let startup = if startup_interrupted {
            starting.await
        } else {
            Ok(())
        };
        let plugins = host.shutdown().await;
        let sockets = backends.shutdown().await;
        tracing::info!("聊天连接已停止");
        if let Some(error) = failure {
            if let Err(shutdown_error) = plugins.and(sockets) {
                tracing::warn!(%shutdown_error, "处理后端故障时退出也发生错误");
            }
            return Err(error);
        }
        startup.and(plugins).and(sockets)
    };
    let forced = async {
        if !first_ctrl_c {
            let _ = interrupts.recv().await;
            tracing::warn!("正在退出，再按一次 Ctrl+C 强制退出");
        }
        let _ = interrupts.recv().await;
    };
    tokio::select! {
        biased;
        _ = forced => Err(ForcedShutdown.into()),
        result = graceful => result,
    }
}
