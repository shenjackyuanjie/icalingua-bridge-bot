use core::str;
use std::sync::LazyLock;
use std::{fmt::Display, path::PathBuf};

use foldhash::HashMap;
use pyo3::types::PyModule;
use pyo3::{
    Bound, IntoPyObject, Py, PyAny, PyErr, Python,
    types::{PyAnyMethods, PyTracebackMethods, PyTuple},
};
use rust_socketio::asynchronous::Client;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::{Level, event};

use crate::MainStatus;
use crate::data_struct::{ica, tailchat};
use crate::error::PyPluginError;
use crate::py::consts::{ica_func, tailchat_func};
use crate::py::{PY_PLUGIN_STORAGE, class};

pub struct PyTaskList {
    lst: Vec<JoinHandle<()>>,
}

impl PyTaskList {
    pub fn new() -> Self { Self { lst: Vec::new() } }

    pub fn push(&mut self, handle: JoinHandle<()>) {
        self.lst.push(handle);
        self.clean_finished();
    }

    pub fn clean_finished(&mut self) { self.lst.retain(|handle| !handle.is_finished()); }

    pub fn len(&self) -> usize { self.lst.len() }

    pub fn is_empty(&self) -> bool { self.lst.is_empty() }

    pub fn cancel_all(&mut self) {
        for handle in self.lst.drain(..) {
            handle.abort();
        }
    }

    pub async fn join_all(&mut self) {
        for handle in self.lst.drain(..) {
            let _ = handle.await;
        }
    }

    pub fn clear(&mut self) { self.lst.clear(); }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum TaskType {
    IcaNewMessage,
    IcaSystemMessage,
    IcaDeleteMessage,
    IcaJoinRequest,
    IcaLeaveMessage,
    TailchatNewMessage,
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
        }
    }
}

impl Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IcaNewMessage => {
                write!(f, "icalingua 的 新消息")
            }
            Self::IcaSystemMessage => {
                write!(f, "icalingua 的 系统消息")
            }
            Self::IcaDeleteMessage => {
                write!(f, "icalingua 的 消息撤回")
            }
            Self::IcaJoinRequest => {
                write!(f, "icalingua 的 加群申请")
            }
            Self::IcaLeaveMessage => {
                write!(f, "icalingua 的 退群消息")
            }
            Self::TailchatNewMessage => {
                write!(f, "Tailchat 的 新消息")
            }
        }
    }
}

pub struct PyTasks {
    tasks: HashMap<TaskType, PyTaskList>,
}

impl PyTasks {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::default(),
        }
    }

    pub fn push(&mut self, task_type: TaskType, handle: JoinHandle<()>) {
        self.tasks.entry(task_type).or_insert_with(PyTaskList::new).push(handle);
    }

    pub fn len(&self, task_type: TaskType) -> usize {
        self.tasks.get(&task_type).map(|v| v.len()).unwrap_or(0)
    }

    pub fn clean_finished(&mut self) {
        let _ = self.tasks.iter_mut().map(|(_, lst)| lst.clean_finished());
    }

    pub async fn join_all(&mut self) {
        self.clean_finished();
        for (task_type, lst) in self.tasks.iter_mut() {
            lst.clean_finished();
            event!(Level::INFO, "正在等待 {task_type} 的任务");
            lst.join_all().await;
        }
    }

    pub fn total_len(&self) -> usize { self.tasks.values().map(|v| v.len()).sum() }

    pub fn is_empty(&self) -> bool { self.total_len() == 0 }
}

/// 全局的 PyTask 存储
///
/// 存储所有任务，方便管理
pub static PY_TASKS: LazyLock<Mutex<PyTasks>> = LazyLock::new(|| Mutex::new(PyTasks::new()));

pub fn get_func<'py>(
    py_module: &Bound<'py, PyAny>,
    name: &'py str,
) -> Result<Bound<'py, PyAny>, PyPluginError> {
    // 获取模块名，失败时使用默认值
    let module_name = py_module
        .getattr("__name__")
        .and_then(|obj| obj.extract::<String>())
        .unwrap_or("module_name_not_found".to_string());

    // 要处理的情况:
    // 1. 有这个函数
    // 2. 没有这个函数
    // 3. 函数不是 Callable
    match py_module.hasattr(name) {
        Ok(contain) => {
            if contain {
                match py_module.getattr(name) {
                    Ok(func) => {
                        if func.is_callable() {
                            Ok(func)
                        } else {
                            Err(PyPluginError::FuncNotCallable(name.to_string(), module_name))
                        }
                    }
                    Err(e) => Err(PyPluginError::CouldNotGetFunc(e, name.to_string(), module_name)),
                }
            } else {
                Err(PyPluginError::FuncNotFound(name.to_string(), module_name))
            }
        }
        Err(e) => Err(PyPluginError::CouldNotGetFunc(e, name.to_string(), module_name)),
    }
}

pub async fn verify_and_reload_plugins() {
    let plugin_path = MainStatus::global_config().py().plugin_path.clone();

    // 先检查是否有插件被删除
    let mut storage = PY_PLUGIN_STORAGE.lock().await;
    let available_path: Vec<PathBuf> = storage.storage.values().map(|p| p.plugin_path()).collect();
    for path in available_path.iter() {
        if !path.exists() {
            event!(Level::INFO, "Python 插件: {:?} 已被删除", path);
            storage.remove_plugin_by_path(path);
        }
    }

    for entry in std::fs::read_dir(plugin_path).unwrap().flatten() {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if ext == "py" {
                match storage.check_and_reload_by_path(&path) {
                    Ok(true) => {
                        event!(Level::INFO, "Python 插件: {:?} 已被重新加载", path);
                    }
                    Err(e) => {
                        event!(Level::ERROR, "Python 插件: {:?} 重载失败: {}", path, e);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn send_warn(py: Python<'_>, e: &PyErr, func_name: &str, plugin_id: &str) {
    event!(
        Level::WARN,
        "error when calling {plugin_id}-func<{}>\ntraceback: {}",
        func_name,
        e.traceback(py)
            .map(|t| t.format().unwrap_or("faild to format traceback".to_string()))
            .unwrap_or("no trackback".to_string())
    );
}

async fn new_task<N>(
    module: &Py<PyModule>,
    func_name: String,
    plugin_id: String,
    args: N,
) -> Option<JoinHandle<()>>
where
    N: for<'py> IntoPyObject<'py, Target = PyTuple> + Send + 'static,
{
    let py_func = { Python::with_gil(|py| module.getattr(py, &func_name).ok()) }?;

    let a = move || {
        Python::with_gil(|py| {
            let _ = py_func
                .call1(py, args)
                .inspect_err(|e| send_warn(py, e, &func_name, &plugin_id));
        })
    };

    Some(tokio::task::spawn_blocking(a))
}

/// 执行 new message 的 python 插件
pub async fn ica_new_message_py(message: &ica::messages::NewMessage, client: &Client) {
    // 验证插件是否改变
    verify_and_reload_plugins().await;

    let storage = PY_PLUGIN_STORAGE.lock().await;
    let plugins = storage.get_enabled_plugins();
    for (plugin_id, plugin) in plugins.iter() {
        let msg = class::ica::NewMessagePy::new(message);
        let client = class::ica::IcaClientPy::new(client);
        let args = (msg, client);
        let task = match new_task(
            &plugin.py_module,
            ica_func::NEW_MESSAGE.to_string(),
            plugin_id.to_string(),
            args,
        )
        .await
        {
            Some(task) => task,
            None => continue,
        };
        PY_TASKS.lock().await.push(TaskType::IcaNewMessage, task);
    }
}

pub async fn ica_system_message_py(message: &ica::messages::NewMessage, client: &Client) {
    verify_and_reload_plugins().await;

    let storage = PY_PLUGIN_STORAGE.lock().await;
    let plugins = storage.get_enabled_plugins();
    for (plugin_id, plugin) in plugins.iter() {
        let msg = class::ica::NewMessagePy::new(message);
        let client = class::ica::IcaClientPy::new(client);
        let args = (msg, client);
        let task = match new_task(
            &plugin.py_module,
            ica_func::SYSTEM_MESSAGE.to_string(),
            plugin_id.to_string(),
            args,
        )
        .await
        {
            Some(task) => task,
            None => continue,
        };
        PY_TASKS.lock().await.push(TaskType::IcaSystemMessage, task);
    }
}

pub async fn ica_delete_message_py(msg_id: ica::MessageId, client: &Client) {
    verify_and_reload_plugins().await;

    let storage = PY_PLUGIN_STORAGE.lock().await;
    let plugins = storage.get_enabled_plugins();
    for (plugin_id, plugin) in plugins.iter() {
        let msg_id = msg_id.clone();
        let client = class::ica::IcaClientPy::new(client);
        let args = (msg_id.clone(), client);
        let task = match new_task(
            &plugin.py_module,
            ica_func::DELETE_MESSAGE.to_string(),
            plugin_id.to_string(),
            args,
        )
        .await
        {
            Some(task) => task,
            None => continue,
        };
        PY_TASKS.lock().await.push(TaskType::IcaDeleteMessage, task);
    }
}

pub async fn tailchat_new_message_py(
    message: &tailchat::messages::ReceiveMessage,
    client: &Client,
) {
    verify_and_reload_plugins().await;

    let storage = PY_PLUGIN_STORAGE.lock().await;
    let plugins = storage.get_enabled_plugins();
    for (plugin_id, plugin) in plugins.iter() {
        let msg = class::tailchat::TailchatReceiveMessagePy::from_recive_message(message);
        let client = class::tailchat::TailchatClientPy::new(client);
        let args = (msg, client);
        let task = match new_task(
            &plugin.py_module,
            tailchat_func::NEW_MESSAGE.to_string(),
            plugin_id.to_string(),
            args,
        )
        .await
        {
            Some(task) => task,
            None => continue,
        };
        PY_TASKS.lock().await.push(TaskType::TailchatNewMessage, task);
    }
}
