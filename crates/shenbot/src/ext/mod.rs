//! 外部进程插件：按配置保留 ID，并只向当前代派发订阅事件。

mod actions;
mod lifecycle;
mod process;
mod supervisor;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::future::join_all;
use ica_ext_sdk::{HostToPlugin, NewMessage};

use crate::config::ExtConfig;

pub use self::actions::ExtHostServices;
use self::lifecycle::{Command, Instance};
use self::process::ProcessRuntime;

#[derive(Debug)]
pub struct ExtPluginError(String);

impl ExtPluginError {
    fn new(message: impl Into<String>) -> Self { Self(message.into()) }
}

impl fmt::Display for ExtPluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

impl Error for ExtPluginError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtPluginState {
    Stopped,
    Starting,
    Active,
    Draining,
    Backoff,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ExtPluginSummary {
    pub id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub enabled: bool,
    pub state: ExtPluginState,
    pub last_error: Option<String>,
    /// 当前路由代号。串行回滚会恢复旧副本的代号。
    pub generation: Option<u64>,
}

pub struct ExtPluginHost {
    instances: BTreeMap<String, Arc<Instance>>,
    closing: Arc<AtomicBool>,
}

impl ExtPluginHost {
    /// 保留所有配置中的 ID（包括未启用的实例），并清理孤儿副本。
    pub fn new(
        config: ExtConfig,
        services: Arc<dyn ExtHostServices>,
    ) -> Result<Self, ExtPluginError> {
        config.validate().map_err(|error| ExtPluginError::new(error.to_string()))?;
        let runtime =
            Arc::new(ProcessRuntime::new(&config.shadow_dir).map_err(|err| {
                ExtPluginError::new(format!("初始化外部插件副本目录失败：{err}"))
            })?);
        let closing = Arc::new(AtomicBool::new(false));
        let timeout = Duration::from_millis(config.handshake_timeout_ms);
        let instances = config
            .plugins
            .into_iter()
            .map(|plugin| {
                let id = plugin.id.clone();
                let grace_ms = plugin.drain_timeout_ms.unwrap_or(config.default_drain_timeout_ms);
                let instance = Instance::new(
                    plugin,
                    timeout,
                    grace_ms,
                    runtime.clone(),
                    services.clone(),
                    closing.clone(),
                );
                (id, Arc::new(instance))
            })
            .collect();
        Ok(Self { instances, closing })
    }

    pub fn contains(&self, id: &str) -> bool { self.instances.contains_key(id) }

    pub fn summaries(&self) -> Vec<ExtPluginSummary> {
        self.instances.values().map(|instance| instance.summary()).collect()
    }

    /// 各实例独立启动；个别失败会留在摘要，且不妨碍其他实例。
    pub async fn start_enabled(&self) {
        join_all(self.instances.values().map(|instance| instance.command(Command::Start))).await;
    }

    pub async fn enable(&self, id: &str) -> Result<(), ExtPluginError> {
        self.command(id, Command::Enable).await
    }

    pub async fn disable(&self, id: &str) -> Result<(), ExtPluginError> {
        self.command(id, Command::Disable).await
    }

    pub async fn reload(&self, id: &str) -> Result<(), ExtPluginError> {
        self.command(id, Command::Reload).await
    }

    async fn command(&self, id: &str, command: Command) -> Result<(), ExtPluginError> {
        self.instances
            .get(id)
            .ok_or_else(|| ExtPluginError::new(format!("未配置外部插件：{id}")))?
            .command(command)
            .await
    }

    /// 投递 ICA 实时事件。握手、回执、停机控制不从此入口转发。
    pub fn dispatch(&self, message: HostToPlugin) { self.dispatch_event(&message, false); }

    /// 由 ICA 宿主显式分类；系统消息只走系统消息订阅。
    pub fn dispatch_system_message(&self, message: NewMessage) {
        self.dispatch_event(&HostToPlugin::NewMessage(message), true);
    }

    fn dispatch_event(&self, event: &HostToPlugin, system_message: bool) {
        for instance in self.instances.values() {
            instance.dispatch(event, system_message);
        }
    }

    /// 停止派发新事件及接纳管理操作，保留在途动作与回执。
    pub fn stop_accepting(&self) { self.closing.store(true, Ordering::Release); }

    /// 先关闭新事件与控制操作入口，再并行排空所有插件。
    pub async fn shutdown(&self) {
        self.stop_accepting();
        join_all(self.instances.values().map(|instance| instance.command(Command::Shutdown))).await;
    }
}
