//! Python 插件钩子的异步调用、归属上下文和任务跟踪。

use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Display,
    path::PathBuf,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use pyo3::{
    Bound, PyAny, PyErr, Python,
    types::{PyAnyMethods, PyTracebackMethods},
};
use rust_socketio::asynchronous::Client;
use tokio::time::Instant;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::{Level, event};

use crate::MainStatus;
use crate::data_struct::ica::all_rooms::JoinRequestRoom;
use crate::data_struct::{ica, tailchat};
use crate::error::PyPluginError;
use crate::py::consts::{ica_func, tailchat_func};
use crate::py::plugin::LifecycleState;
use crate::py::{PY_PLUGIN_STORAGE, class};

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
    IcaLeaveMessage,
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
            TaskType::IcaLeaveMessage => ica_func::LEAVE_MESSAGE,
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

#[derive(Debug, Clone, Copy)]
pub enum TaskKind {
    Hook(TaskType),
    Scheduler,
}

struct TaskRecord {
    kind: TaskKind,
    handle: JoinHandle<()>,
}

pub struct PyTasks {
    tasks: HashMap<PluginIdentity, Vec<TaskRecord>>,
}

impl PyTasks {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    pub fn register(&mut self, identity: PluginIdentity, kind: TaskKind, handle: JoinHandle<()>) {
        self.clean_finished();
        self.tasks.entry(identity).or_default().push(TaskRecord { kind, handle });
    }

    pub fn abort_schedulers(&mut self, identity: &PluginIdentity) {
        if let Some(records) = self.tasks.get_mut(identity) {
            for record in records.iter() {
                if matches!(record.kind, TaskKind::Scheduler) {
                    record.handle.abort();
                }
            }
        }
        self.clean_finished();
    }

    pub fn hooks_finished(&mut self, identity: &PluginIdentity) -> bool {
        self.clean_finished();
        self.tasks.get(identity).is_none_or(|records| {
            records.iter().all(|record| !matches!(record.kind, TaskKind::Hook(_)))
        })
    }

    pub fn clean_finished(&mut self) {
        for records in self.tasks.values_mut() {
            records.retain(|record| !record.handle.is_finished());
        }
        self.tasks.retain(|_, records| !records.is_empty());
    }

    pub fn total_len(&mut self) -> usize {
        self.clean_finished();
        self.tasks.values().map(Vec::len).sum()
    }

    pub fn is_empty(&mut self) -> bool { self.total_len() == 0 }

    fn drain_all(&mut self) -> Vec<TaskRecord> {
        self.tasks.drain().flat_map(|(_, records)| records).collect()
    }
}

pub static PY_TASKS: LazyLock<Mutex<PyTasks>> = LazyLock::new(|| Mutex::new(PyTasks::new()));
static PLUGIN_SCAN_WARNING_ACTIVE: AtomicBool = AtomicBool::new(false);

pub async fn abort_schedulers(identity: &PluginIdentity) {
    PY_TASKS.lock().await.abort_schedulers(identity);
}

pub async fn wait_for_hooks(identity: &PluginIdentity, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if PY_TASKS.lock().await.hooks_finished(identity) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub async fn stop_all_tasks() {
    let records = PY_TASKS.lock().await.drain_all();
    for record in &records {
        if matches!(record.kind, TaskKind::Scheduler) {
            record.handle.abort();
        }
    }
    for record in records {
        if let TaskKind::Hook(task_type) = record.kind {
            event!(Level::DEBUG, "等待 Python hook {task_type}");
        }
        let _ = record.handle.await;
    }
}

pub fn get_func<'py>(
    py_module: &Bound<'py, PyAny>,
    name: &'py str,
) -> Result<Bound<'py, PyAny>, PyPluginError> {
    let module_name = py_module
        .getattr("__name__")
        .and_then(|obj| obj.extract::<String>())
        .unwrap_or_else(|_| "module_name_not_found".to_string());
    if !py_module
        .hasattr(name)
        .map_err(|e| PyPluginError::CouldNotGetFunc(e, name.to_string(), module_name.clone()))?
    {
        return Err(PyPluginError::FuncNotFound(name.to_string(), module_name));
    }
    let func = py_module
        .getattr(name)
        .map_err(|e| PyPluginError::CouldNotGetFunc(e, name.to_string(), module_name.clone()))?;
    if !func.is_callable() {
        return Err(PyPluginError::FuncNotCallable(name.to_string(), module_name));
    }
    Ok(func)
}

pub async fn verify_and_reload_plugins() {
    let plugin_path = PathBuf::from(MainStatus::global_config().py().plugin_path.clone());
    match crate::py::storage::scan_plugins(&plugin_path).await {
        Ok(()) => {
            PLUGIN_SCAN_WARNING_ACTIVE.store(false, Ordering::Release);
        }
        Err(error) => {
            if !PLUGIN_SCAN_WARNING_ACTIVE.swap(true, Ordering::AcqRel) {
                event!(
                    Level::WARN,
                    "Python 插件目录 {:?} 扫描失败，继续使用当前插件: {error}",
                    plugin_path
                );
            }
        }
    }
}

fn send_warn(py: Python<'_>, error: &PyErr, func_name: &str, plugin_id: &str) {
    event!(
        Level::WARN,
        "error when calling {plugin_id}-func<{func_name}>\ntraceback: {}{error}",
        error
            .traceback(py)
            .map(|traceback| traceback.format().unwrap_or_else(|_| "traceback 格式化失败".into()))
            .unwrap_or_else(|| "no traceback".into())
    );
}

async fn call_plugins<F, A>(task_type: TaskType, func_name: &str, build_args: F)
where
    F: Fn() -> A,
    A: for<'py> pyo3::call::PyCallArgs<'py> + Send + 'static,
{
    verify_and_reload_plugins().await;

    let snapshots = {
        let storage = PY_PLUGIN_STORAGE.lock().await;
        Python::attach(|py| {
            storage
                .storage
                .iter()
                .filter(|(_, plugin)| plugin.state() == LifecycleState::Active)
                .map(|(plugin_id, plugin)| {
                    (
                        PluginIdentity::new(plugin_id.clone(), plugin.generation()),
                        plugin.py_module.clone_ref(py),
                    )
                })
                .collect::<Vec<_>>()
        })
    };

    for (identity, module) in snapshots {
        let py_func = Python::attach(|py| module.getattr(py, func_name).ok());
        let Some(py_func) = py_func else {
            continue;
        };
        let args = build_args();
        let func_name = func_name.to_string();
        let task_identity = identity.clone();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            if gate_rx.await.is_err() {
                return;
            }
            let _ = tokio::task::spawn_blocking(move || {
                with_plugin_context(task_identity.clone(), || {
                    Python::attach(|py| {
                        let _ = py_func.call1(py, args).inspect_err(|error| {
                            send_warn(py, error, &func_name, &task_identity.plugin_id)
                        });
                    });
                });
            })
            .await;
        });

        let registered = {
            let storage = PY_PLUGIN_STORAGE.lock().await;
            let active = storage.storage.get(&identity.plugin_id).is_some_and(|plugin| {
                plugin.state() == LifecycleState::Active
                    && plugin.generation() == identity.generation
            });
            if active {
                PY_TASKS
                    .lock()
                    .await
                    .register(identity.clone(), TaskKind::Hook(task_type), handle);
                true
            } else {
                handle.abort();
                false
            }
        };
        if registered {
            let _ = gate_tx.send(());
        }
    }
}

pub async fn ica_new_message_py(message: &ica::messages::NewMessage, client: &Client) {
    call_plugins(TaskType::IcaNewMessage, ica_func::NEW_MESSAGE, || {
        (class::ica::NewMessagePy::new(message), class::ica::IcaClientPy::new(client))
    })
    .await;
}

pub async fn ica_system_message_py(message: &ica::messages::NewMessage, client: &Client) {
    call_plugins(TaskType::IcaSystemMessage, ica_func::SYSTEM_MESSAGE, || {
        (class::ica::NewMessagePy::new(message), class::ica::IcaClientPy::new(client))
    })
    .await;
}

pub async fn ica_delete_message_py(msg_id: ica::MessageId, client: &Client) {
    call_plugins(TaskType::IcaDeleteMessage, ica_func::DELETE_MESSAGE, || {
        (msg_id.clone(), class::ica::IcaClientPy::new(client))
    })
    .await;
}

pub async fn ica_join_request_py(event: JoinRequestRoom, client: &Client) {
    call_plugins(TaskType::IcaJoinRequest, ica_func::JOIN_REQUEST, || {
        (class::ica::IcaJoinRequestPy::new(&event), class::ica::IcaClientPy::new(client))
    })
    .await;
}

pub async fn tailchat_new_message_py(
    message: &tailchat::messages::ReceiveMessage,
    client: &Client,
) {
    call_plugins(TaskType::TailchatNewMessage, tailchat_func::NEW_MESSAGE, || {
        (
            class::tailchat::TailchatReceiveMessagePy::from_recive_message(message),
            class::tailchat::TailchatClientPy::new(client),
        )
    })
    .await;
}

#[cfg(test)]
mod task_tests {
    use std::time::Duration;

    use super::{
        PY_TASKS, PluginIdentity, TaskKind, TaskType, abort_schedulers, current_plugin_identity,
        wait_for_hooks, with_plugin_context,
    };

    #[test]
    fn plugin_context_is_scoped_and_restored() {
        let outer = PluginIdentity::new("outer", 1);
        let inner = PluginIdentity::new("inner", 2);
        assert!(current_plugin_identity().is_none());
        with_plugin_context(outer.clone(), || {
            assert_eq!(current_plugin_identity(), Some(outer.clone()));
            with_plugin_context(inner.clone(), || {
                assert_eq!(current_plugin_identity(), Some(inner));
            });
            assert_eq!(current_plugin_identity(), Some(outer));
        });
        assert!(current_plugin_identity().is_none());
    }

    #[tokio::test]
    async fn scheduler_is_aborted_but_hooks_are_drained() {
        let identity = PluginIdentity::new("task-test", 1);
        let scheduler = tokio::spawn(std::future::pending::<()>());
        PY_TASKS.lock().await.register(identity.clone(), TaskKind::Scheduler, scheduler);
        abort_schedulers(&identity).await;
        tokio::task::yield_now().await;
        assert!(wait_for_hooks(&identity, Duration::from_millis(20)).await);

        let hook = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(30)).await;
        });
        PY_TASKS.lock().await.register(
            identity.clone(),
            TaskKind::Hook(TaskType::IcaNewMessage),
            hook,
        );
        assert!(!wait_for_hooks(&identity, Duration::from_millis(5)).await);
        assert!(wait_for_hooks(&identity, Duration::from_millis(100)).await);
    }
}
