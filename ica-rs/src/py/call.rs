//! Python hook 归属和快照分发；事件路径没有磁盘扫描或解释器锁。
use super::{
    class,
    consts::{ica_func, tailchat_func},
    host::PluginHost,
};
use crate::data_struct::ica::all_rooms::JoinRequestRoom;
use crate::data_struct::{ica, tailchat};
use pyo3::types::PyTypeMethods;
use pyo3::{Bound, IntoPyObject, PyAny, PyErr, Python};
use rust_socketio::asynchronous::Client;
use serde_json::Value;
use std::{cell::RefCell, fmt::Display, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginIdentity {
    pub plugin_id: String,
    pub generation: u64,
}

impl PluginIdentity {
    pub fn new(plugin_id: impl Into<String>, generation: u64) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            generation,
        }
    }
}

thread_local! {
    static CURRENT_PLUGIN: RefCell<Option<PluginIdentity>> = const { RefCell::new(None) };
}

struct PluginContextGuard(Option<PluginIdentity>);

impl Drop for PluginContextGuard {
    fn drop(&mut self) {
        CURRENT_PLUGIN.with(|current| {
            current.replace(self.0.take());
        });
    }
}

/// 在当前线程设置插件归属，供生命周期钩子和 `Scheduler.start()` 使用。
pub(crate) fn with_plugin_context<T>(identity: PluginIdentity, callback: impl FnOnce() -> T) -> T {
    let previous = CURRENT_PLUGIN.with(|current| current.replace(Some(identity)));
    let _guard = PluginContextGuard(previous);
    callback()
}

pub(crate) fn current_plugin_identity() -> Option<PluginIdentity> {
    CURRENT_PLUGIN.with(|current| current.borrow().clone())
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum TaskType {
    IcaNewMessage,
    IcaSystemMessage,
    IcaDeleteMessage,
    IcaJoinRequest,
    IcaMessageUpdated,
    IcaBridgeEvent,
    TailchatNewMessage,
    SchedulerCallback,
}

impl TaskType {
    pub fn py_func_str(&self) -> &'static str {
        match self {
            TaskType::IcaNewMessage => ica_func::NEW_MESSAGE,
            TaskType::IcaSystemMessage => ica_func::SYSTEM_MESSAGE,
            TaskType::IcaDeleteMessage => ica_func::DELETE_MESSAGE,
            TaskType::IcaJoinRequest => ica_func::JOIN_REQUEST,
            TaskType::IcaMessageUpdated => "on_ica_message_updated",
            TaskType::IcaBridgeEvent => "on_ica_bridge_event",
            TaskType::TailchatNewMessage => tailchat_func::NEW_MESSAGE,
            TaskType::SchedulerCallback => "scheduler_callback",
        }
    }
}

impl Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.py_func_str())
    }
}

pub async fn call_plugins<F, A>(
    host: &Arc<PluginHost>,
    task_type: TaskType,
    _func_name: &str,
    build_args: F,
) where
    F: Fn() -> A,
    A: for<'py> pyo3::call::PyCallArgs<'py> + Send + 'static,
{
    if !host.is_accepting() {
        return;
    }
    for (identity, callback) in host.snapshots(task_type).await {
        let owner = identity.clone();
        host.enqueue_with(identity, || {
            let args = build_args();
            move || Python::attach(|py| {
                if let Err(error) = callback.call1(py, args) {
                    if matches!(task_type, TaskType::IcaBridgeEvent | TaskType::IcaMessageUpdated) {
                        // 登录挑战或消息数据可能出现在插件异常文本中，只记录异常种类。
                        tracing::warn!(plugin_id = %owner.plugin_id, hook = %task_type,
                            error_type = ?error.get_type(py).name(), "Python 业务事件 hook 失败");
                    } else {
                        tracing::warn!(plugin_id = %owner.plugin_id, hook = %task_type, error = %error, "Python hook 失败");
                    }
                }
            })
        }).await;
    }
}

pub async fn ica_new_message_py(
    host: &Arc<PluginHost>,
    message: &ica::messages::NewMessage,
    client: &Client,
) {
    call_plugins(host, TaskType::IcaNewMessage, ica_func::NEW_MESSAGE, || {
        (
            class::ica::NewMessagePy::new(message, &host.context),
            class::ica::IcaClientPy::new(client, host),
        )
    })
    .await;
}

pub async fn ica_system_message_py(
    host: &Arc<PluginHost>,
    message: &ica::messages::NewMessage,
    client: &Client,
) {
    call_plugins(host, TaskType::IcaSystemMessage, ica_func::SYSTEM_MESSAGE, || {
        (
            class::ica::NewMessagePy::new(message, &host.context),
            class::ica::IcaClientPy::new(client, host),
        )
    })
    .await;
}

pub async fn ica_delete_message_py(
    host: &Arc<PluginHost>,
    msg_id: ica::MessageId,
    client: &Client,
) {
    call_plugins(host, TaskType::IcaDeleteMessage, ica_func::DELETE_MESSAGE, || {
        (msg_id.clone(), class::ica::IcaClientPy::new(client, host))
    })
    .await;
}

pub async fn ica_join_request_py(host: &Arc<PluginHost>, event: JoinRequestRoom, client: &Client) {
    call_plugins(host, TaskType::IcaJoinRequest, ica_func::JOIN_REQUEST, || {
        (
            class::ica::IcaJoinRequestPy::new(&event),
            class::ica::IcaClientPy::new(client, host),
        )
    })
    .await;
}

/// 在工作线程持有解释器时才把拥有的 JSON 快照转为 Python dict/list。
struct JsonHookValue(Value);

impl<'py> IntoPyObject<'py> for JsonHookValue {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        super::json::json_to_py(py, &self.0).map(|value| value.into_bound(py))
    }
}

/// 消息变更不作为新消息重放；patch 保持 Bridge 的 Partial<Message> 结构。
pub async fn ica_message_updated_py(
    host: &Arc<PluginHost>,
    client: &Client,
    room_id: ica::RoomId,
    message_id: &str,
    patch: &Value,
) {
    call_plugins(host, TaskType::IcaMessageUpdated, "on_ica_message_updated", || {
        (
            class::ica::IcaClientPy::new(client, host),
            room_id,
            message_id.to_owned(),
            JsonHookValue(patch.clone()),
        )
    })
    .await;
}

/// 非 GUI Bridge 事件保留 wire 名称与参数列表，不把账号/挑战内容写进日志。
pub async fn ica_bridge_event_py(
    host: &Arc<PluginHost>,
    client: &Client,
    event: &str,
    args: Vec<Value>,
) {
    call_plugins(host, TaskType::IcaBridgeEvent, "on_ica_bridge_event", || {
        (
            class::ica::IcaClientPy::new(client, host),
            event.to_owned(),
            JsonHookValue(Value::Array(args.clone())),
        )
    })
    .await;
}

pub async fn tailchat_new_message_py(
    host: &Arc<PluginHost>,
    message: &tailchat::messages::ReceiveMessage,
    client: &Client,
) {
    call_plugins(host, TaskType::TailchatNewMessage, tailchat_func::NEW_MESSAGE, || {
        (
            class::tailchat::TailchatReceiveMessagePy::from_recive_message(message, &host.context),
            class::tailchat::TailchatClientPy::new(client, host),
        )
    })
    .await;
}
