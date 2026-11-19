//! 协议层和插件后端之间的中立宿主。
pub mod plugin;
pub(crate) mod runtime;

use crate::{
    context::BotContext,
    data_struct::{
        ica::{MessageId, RoomId, all_rooms::JoinRequestRoom, messages::NewMessage},
        tailchat::messages::ReceiveMessage,
    },
    ica::connection::{Connection, ConnectionState},
    py::{self, host::PluginHost as PyPluginHost},
};
use plugin::{PluginKind, PluginRegistry, PluginSummary};
use rust_socketio::asynchronous::Client;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Semaphore;

pub struct BotHost {
    pub context: Arc<BotContext>,
    pub registry: Arc<PluginRegistry>,
    py: Option<Arc<PyPluginHost>>,
    accepting: AtomicBool,
    pub(crate) fetch_slot: Arc<Semaphore>,
    pub(crate) command_slots: Arc<Semaphore>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PluginAction {
    Enable,
    Disable,
    Reload,
}

#[derive(Debug)]
pub(crate) enum PluginChange {
    NotFound,
    Unchanged,
    Changed,
    Failed(anyhow::Error),
}

impl BotHost {
    pub fn new(context: Arc<BotContext>) -> Arc<Self> {
        let registry = Arc::new(PluginRegistry::default());
        let py = context
            .config
            .check_py()
            .then(|| PyPluginHost::new(context.clone(), registry.clone()));
        Arc::new(Self {
            context,
            registry,
            py,
            accepting: AtomicBool::new(true),
            fetch_slot: Arc::new(Semaphore::new(1)),
            command_slots: Arc::new(Semaphore::new(16)),
        })
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        if let Some(py) = &self.py {
            py::init_py(py).await?;
        }
        Ok(())
    }

    pub fn is_accepting(&self) -> bool { self.accepting.load(Ordering::Acquire) }

    /// 关闭新命令/事件的入口，但保留连接供已经接纳的插件动作使用。
    pub fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Release);
        if let Some(py) = &self.py {
            py.stop_accepting();
        }
    }

    /// 排空插件；调用方在本阶段结束后再停止 Socket.IO。
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.stop_accepting();
        if let Some(py) = &self.py {
            py.shutdown().await?;
        }
        Ok(())
    }

    pub fn ica_connection(&self) -> &Connection<Client> { &self.context.ica_connection }

    #[cfg(test)]
    pub(crate) fn python(&self) -> Option<&Arc<PyPluginHost>> { self.py.as_ref() }

    pub(crate) fn python_version(&self) -> Option<&'static str> {
        self.py.as_ref().map(|_| py::runtime_version())
    }

    pub async fn plugins(&self) -> Vec<PluginSummary> {
        let mut plugins = if let Some(py) = &self.py {
            py.plugins().await
        } else {
            Vec::new()
        };
        plugins.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        plugins
    }

    pub(crate) async fn change_plugin(&self, id: &str, action: PluginAction) -> PluginChange {
        if !self.is_accepting() {
            return PluginChange::Failed(anyhow::anyhow!("插件服务正在关闭"));
        }
        let Some(PluginKind::Py) = self.registry.owner(id) else {
            return PluginChange::NotFound;
        };
        let Some(py) = &self.py else {
            return PluginChange::NotFound;
        };
        let Some(enabled) = py.plugin_status(id).await else {
            return PluginChange::NotFound;
        };
        let result = match (action, enabled) {
            (PluginAction::Enable, true) | (PluginAction::Disable, false) => {
                return PluginChange::Unchanged;
            }
            (PluginAction::Enable, false) => py.set_plugin_enabled(id, true).await,
            (PluginAction::Disable, true) => py.set_plugin_enabled(id, false).await,
            (PluginAction::Reload, _) => py.reload_plugin(id).await,
        };
        match result {
            Ok(()) => PluginChange::Changed,
            Err(error) => PluginChange::Failed(error),
        }
    }

    fn accepting_python(&self) -> Option<&Arc<PyPluginHost>> {
        self.py.as_ref().filter(|_| self.is_accepting())
    }

    pub(crate) async fn dispatch_ica_candidate<T: Clone>(
        &self,
        connection: &ConnectionState<T>,
        generation: u64,
        message: &NewMessage,
    ) {
        if let Some(py) = self.accepting_python() {
            py.dispatch_ica_candidate(connection, generation, message).await;
        }
    }

    /// 实时、经过 sender 过滤的消息入口；与候选、历史缓存和 wire hook 分离。
    pub(crate) async fn dispatch_ica_message(
        &self,
        message: &NewMessage,
        client: &Client,
        generation: u64,
    ) {
        if self.ica_connection().with_open_generation(generation, || ()).is_none() {
            return;
        }
        if let Some(py) = self.accepting_python() {
            py.dispatch_ica_message(message, client).await;
        }
    }

    pub(crate) async fn dispatch_ica_bridge_event(
        &self,
        client: &Client,
        name: &str,
        args: impl FnOnce() -> Vec<Value>,
    ) {
        if let Some(py) = self.accepting_python() {
            py::call::ica_bridge_event_py(py, client, name, args()).await;
        }
    }

    pub(crate) async fn dispatch_ica_delete(&self, id: MessageId, client: &Client) {
        if let Some(py) = self.accepting_python() {
            py::call::ica_delete_message_py(py, id, client).await;
        }
    }

    pub(crate) async fn dispatch_ica_update(
        &self,
        client: &Client,
        room: RoomId,
        id: &str,
        patch: &Value,
    ) {
        if let Some(py) = self.accepting_python() {
            py::call::ica_message_updated_py(py, client, room, id, patch).await;
        }
    }

    pub(crate) async fn dispatch_ica_join_request(&self, event: JoinRequestRoom, client: &Client) {
        if let Some(py) = self.accepting_python() {
            py::call::ica_join_request_py(py, event, client).await;
        }
    }

    pub(crate) async fn dispatch_tailchat_message(
        &self,
        message: &ReceiveMessage,
        client: &Client,
    ) {
        if let Some(py) = self.accepting_python() {
            py::call::tailchat_new_message_py(py, message, client).await;
        }
    }
}
