//! 监管单代进程。无论新消息路由是否已经切走，都维护旧代的动作通道。

use std::sync::{Arc, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::stream::{FuturesUnordered, StreamExt};
use ica_ext_sdk::PluginToHost;
use tokio::sync::watch;
use tokio::time::{Instant, sleep_until};

use super::ExtHostServices;
use super::actions::ActionContext;
use super::lifecycle::Instance;
use super::process::{PluginChild, PluginProcess, PluginStdout};

const MAX_PENDING_ACTIONS: usize = 16;

pub(super) struct Monitor {
    pub(super) id: String,
    pub(super) generation: u64,
    pub(super) instance: Weak<Instance>,
    pub(super) run: Arc<()>,
    pub(super) process: PluginProcess,
    pub(super) services: Arc<dyn ExtHostServices>,
    pub(super) force: watch::Receiver<bool>,
    pub(super) finished: watch::Sender<bool>,
    pub(super) write_timeout: Duration,
}

// 监管任务最终可能通过 exited 启动下一代。擦除 future 类型，让各代相互独立。
pub(super) fn run(monitor: Monitor) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        let Monitor {
            id,
            generation,
            instance,
            run,
            process,
            services,
            force,
            finished,
            write_timeout,
        } = monitor;
        let PluginProcess {
            stdin,
            stdout,
            child,
        } = process;
        let context = ActionContext {
            plugin_id: id.clone(),
            instance: instance.clone(),
            run: run.clone(),
            stdin,
            services,
            write_timeout,
        };
        let description = observe(stdout, child, context, force).await;
        tracing::debug!(plugin_id = %id, generation, %description, "外部插件进程已退出");
        // 先放行正在等待此进程的退场操作；退出通知需要获取同一个管理锁。
        let _ = finished.send(true);
        if let Some(instance) = instance.upgrade() {
            instance.exited(&run, description).await;
        }
    })
}

async fn observe(
    mut stdout: PluginStdout,
    mut child: PluginChild,
    context: ActionContext,
    mut force: watch::Receiver<bool>,
) -> String {
    let mut pending = FuturesUnordered::new();
    let mut exit: Option<(String, Instant)> = None;
    let failure = loop {
        let exit_deadline = exit.as_ref().map_or_else(Instant::now, |(_, deadline)| *deadline);
        tokio::select! {
            biased;
            _ = force.changed() => break "外部插件已强制终止".into(),
            result = child.wait(), if exit.is_none() => match result {
                Ok(status) => exit = Some((format!("外部插件退出：{status}"), Instant::now() + context.write_timeout)),
                Err(error) => break format!("等待外部插件退出失败：{error}"),
            },
            _ = sleep_until(exit_deadline), if exit.is_some() => break exit.take().expect("已观察到退出").0,
            Some(()) = pending.next(), if !pending.is_empty() => {},
            event = stdout.recv(), if pending.len() < MAX_PENDING_ACTIONS => match event {
                Ok(Some(PluginToHost::Hello(_))) => break "外部插件在握手后重复发送 hello".into(),
                Ok(Some(action)) => pending.push(context.execute(action)),
                Ok(None) => {
                    if let Some((description, _)) = exit.take() { return description; }
                    break "外部插件关闭了协议输出".into();
                }
                Err(error) => break format!("外部插件输出协议错误：{error}"),
            },
        }
    };
    drop(pending);
    if let Err(error) = child.kill().await {
        tracing::warn!(plugin_id = %context.plugin_id, %error, "终止外部插件时出错");
    }
    failure
}
