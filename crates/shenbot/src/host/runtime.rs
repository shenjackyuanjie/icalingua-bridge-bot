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

/// 控制台 Ctrl+C 的第二条通道。
///
/// Windows 的控制台处理器后注册先分派，Python 解释器或插件线程只要登记过自己的
/// 处理器，就会先吃掉 CTRL_C_EVENT，让 Tokio 那份订阅永远等不到事件。这里额外登记
/// 一份在解释器就绪后安装的处理器，保证排在最前面，并把事件转发给运行时。
#[cfg(windows)]
mod console {
    use std::sync::OnceLock;
    use tokio::sync::Notify;
    use windows_sys::Win32::System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler};

    static CANCELLED: OnceLock<Notify> = OnceLock::new();

    pub(super) fn cancelled() -> &'static Notify { CANCELLED.get_or_init(Notify::new) }

    /// 处理器运行在系统线程上，只能做无阻塞的通知，不能记日志。
    unsafe extern "system" fn handler(event: u32) -> i32 {
        if event == CTRL_C_EVENT {
            cancelled().notify_one();
            1
        } else {
            0
        }
    }

    pub(super) fn install() -> std::io::Result<()> {
        // SAFETY: 处理器是进程内的静态函数，注册后一直有效；只加不移除。
        if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Ctrl+C 订阅：Tokio 的信号流，加上 Windows 上自己登记的处理器。
struct Interrupts {
    #[cfg(windows)]
    native: tokio::signal::windows::CtrlC,
    #[cfg(unix)]
    native: tokio::signal::unix::Signal,
}

impl Interrupts {
    fn new() -> std::io::Result<Self> {
        #[cfg(windows)]
        return Ok(Self {
            native: tokio::signal::windows::ctrl_c()?,
        });
        #[cfg(unix)]
        return Ok(Self {
            native: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
        });
    }

    /// 插件启动完成后再登记：此时解释器与插件模块都已就绪，晚注册才能排在最前。
    fn after_python(&self) {
        #[cfg(windows)]
        if let Err(error) = console::install() {
            tracing::warn!(%error, "无法登记控制台 Ctrl+C 处理器，继续使用 Tokio 的订阅");
        }
    }

    async fn recv(&mut self) {
        #[cfg(windows)]
        tokio::select! {
            biased;
            _ = console::cancelled().notified() => {}
            _ = self.native.recv() => {}
        }
        #[cfg(unix)]
        let _ = self.native.recv().await;
    }
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
    let mut interrupts = Interrupts::new()?;
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
            interrupts.after_python();
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
