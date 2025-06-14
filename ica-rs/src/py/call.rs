use core::str;
use std::path::PathBuf;
use std::sync::LazyLock;

use foldhash::HashMap;
use pyo3::{
    Bound, PyAny, Python,
    types::{PyAnyMethods, PyTracebackMethods},
};
use rust_socketio::asynchronous::Client;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::{Level, event, info, warn};

use crate::MainStatus;
use crate::data_struct::{ica, tailchat};
use crate::error::PyPluginError;
use crate::py::consts::{ica_func, tailchat_func};
use crate::py::{PyPlugin, PyStatus, class};

pub struct PyTaskList {
    lst: Vec<JoinHandle<()>>,
}

impl PyTaskList {
    pub fn new() -> Self { Self { lst: Vec::new() } }

    pub fn push(&mut self, handle: tokio::task::JoinHandle<()>) {
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
    IcaDeleteMessage,
    TailchatNewMessage,
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

    pub fn total_len(&self) -> usize { self.tasks.values().map(|v| v.len()).sum() }
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

pub fn verify_and_reload_plugins() {
    let mut need_reload_files: Vec<PathBuf> = Vec::new();
    let plugin_path = MainStatus::global_config().py().plugin_path.clone();

    // 先检查是否有插件被删除
    for path in PyStatus::get().files.keys() {
        if !path.exists() {
            event!(Level::INFO, "Python 插件: {:?} 已被删除", path);
            PyStatus::get_mut().delete_file(path);
        }
    }

    for entry in std::fs::read_dir(plugin_path).unwrap().flatten() {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if ext == "py" && !PyStatus::get().verify_file(&path) {
                need_reload_files.push(path);
            }
        }
    }

    if need_reload_files.is_empty() {
        return;
    }
    event!(Level::INFO, "更改列表: {:?}", need_reload_files);
    let plugins = PyStatus::get_mut();
    for reload_file in need_reload_files {
        if let Some(plugin) = plugins.files.get_mut(&reload_file) {
            plugin.reload_from_file();
            event!(Level::INFO, "重载 Python 插件: {:?} 完成", reload_file);
        } else {
            match PyPlugin::new_from_path(&reload_file) {
                Some(plugin) => {
                    plugins.add_file(reload_file.clone(), plugin);
                    info!("加载 Python 插件: {:?} 完成", reload_file);
                }
                None => {
                    warn!("加载 Python 插件: {:?} 失败", reload_file);
                }
            }
        }
    }
}

macro_rules! call_py_func {
    ($args:expr, $plugin:expr, $plugin_path:expr, $func_name:expr, $client:expr) => {
        tokio::spawn(async move {
            Python::with_gil(|py| {
                if let Ok(py_func) = get_func($plugin.py_module.bind(py), $func_name) {
                    if let Err(py_err) = py_func.call1($args) {
                        let e = PyPluginError::FuncCallError(
                            py_err,
                            $func_name.to_string(),
                            $plugin_path.to_string_lossy().to_string(),
                        );
                        event!(
                            Level::WARN,
                            "failed to call function<{}>: {}\ntraceback: {}",
                            $func_name,
                            e,
                            // 获取 traceback
                            match &e {
                                PyPluginError::FuncCallError(py_err, _, _) => match py_err.traceback(py) {
                                    Some(traceback) => match traceback.format() {
                                        Ok(trace) => trace,
                                        Err(trace_e) => format!("failed to format traceback: {:?}", trace_e),
                                    },
                                    None => "no traceback".to_string(),
                                },
                                _ => unreachable!(),
                            }
                        );
                    }
                }
            })
        })
    };
}

/// 执行 new message 的 python 插件
pub async fn ica_new_message_py(message: &ica::messages::NewMessage, client: &Client) {
    // 验证插件是否改变
    verify_and_reload_plugins();

    let plugins = PyStatus::get();
    for (path, plugin) in plugins.files.iter().filter(|(_, plugin)| plugin.enabled) {
        let msg = class::ica::NewMessagePy::new(message);
        let client = class::ica::IcaClientPy::new(client);
        let args = (msg, client);
        let task = call_py_func!(args, plugin, path, ica_func::NEW_MESSAGE, client);
        PY_TASKS.lock().await.push(TaskType::IcaNewMessage, task);
    }
}

pub async fn ica_delete_message_py(msg_id: ica::MessageId, client: &Client) {
    verify_and_reload_plugins();

    let plugins = PyStatus::get();
    for (path, plugin) in plugins.files.iter().filter(|(_, plugin)| plugin.enabled) {
        let msg_id = msg_id.clone();
        let client = class::ica::IcaClientPy::new(client);
        let args = (msg_id.clone(), client);
        let task = call_py_func!(args, plugin, path, ica_func::DELETE_MESSAGE, client);
        PY_TASKS.lock().await.push(TaskType::IcaDeleteMessage, task);
    }
}

pub async fn tailchat_new_message_py(
    message: &tailchat::messages::ReceiveMessage,
    client: &Client,
) {
    verify_and_reload_plugins();

    let plugins = PyStatus::get();
    for (path, plugin) in plugins.files.iter().filter(|(_, plugin)| plugin.enabled) {
        let msg = class::tailchat::TailchatReceiveMessagePy::from_recive_message(message);
        let client = class::tailchat::TailchatClientPy::new(client);
        let args = (msg, client);
        let task = call_py_func!(args, plugin, path, tailchat_func::NEW_MESSAGE, client);
        PY_TASKS.lock().await.push(TaskType::IcaNewMessage, task);
    }
}
