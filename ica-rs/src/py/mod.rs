pub mod call;
pub mod class;
pub mod consts;
pub mod init;
pub mod plugin;
pub mod storage;

use std::{ffi::CStr, sync::LazyLock};

use colored::Colorize;
use pyo3::{PyErr, Python, types::PyTracebackMethods};
use tokio::sync::Mutex;
use tracing::{Level, event, span};

use crate::error::PyPluginError;

use storage::PyPluginStorage;

/// 全局的插件存储
pub static PY_PLUGIN_STORAGE: LazyLock<Mutex<PyPluginStorage>> =
    LazyLock::new(|| Mutex::new(PyPluginStorage::new()));

/// Python 侧初始化
pub async fn init_py() {
    // 从 全局配置中获取 python 插件路径
    let span = span!(Level::INFO, "py init");
    let _enter = span.enter();

    event!(Level::INFO, "开始初始化 python");

    // 注册东西
    class::regist_class();

    let mut storage = PY_PLUGIN_STORAGE.lock().await;
    storage.load_plugins();

    event!(Level::DEBUG, "python 插件列表: {}", storage.display_plugins());

    event!(Level::INFO, "python 初始化完成")
}

pub async fn post_py() -> anyhow::Result<()> {
    let storage = PY_PLUGIN_STORAGE.lock().await;
    storage.sync_status_to_file();

    stop_tasks().await?;
    Ok(())
}

async fn stop_tasks() -> Result<(), PyPluginError> {
    if call::PY_TASKS.lock().await.is_empty() {
        return Ok(());
    }
    let waiter = tokio::spawn(async {
        call::PY_TASKS.lock().await.join_all().await;
    });
    tokio::select! {
        _ = waiter => {
            event!(Level::INFO, "Python 任务完成");
            Ok(())
        }
        _ = tokio::signal::ctrl_c() => {
            event!(Level::WARN, "正在强制结束 Python 任务");
            Err(PyPluginError::PluginNotStopped)
        }
    }
}

/// code from: pyo3-ffi
pub const fn c_str_from_str(s: &str) -> &CStr {
    // TODO: Replace this implementation with `CStr::from_bytes_with_nul` when MSRV above 1.72.
    let bytes = s.as_bytes();
    let len = bytes.len();
    assert!(
        !bytes.is_empty() && bytes[bytes.len() - 1] == b'\0',
        "string is not nul-terminated"
    );
    let mut i = 0;
    let non_null_len = len - 1;
    while i < non_null_len {
        assert!(bytes[i] != b'\0', "string contains null bytes");
        i += 1;
    }

    unsafe { CStr::from_bytes_with_nul_unchecked(bytes) }
}

/// 获取 python 错误信息
pub fn get_py_err_traceback(py_err: &PyErr) -> String {
    Python::with_gil(|py| match py_err.traceback(py) {
        Some(traceback) => traceback.format().unwrap_or_else(|e| format!("{e:?}")),
        None => "".to_string(),
    })
    .red()
    .to_string()
}
