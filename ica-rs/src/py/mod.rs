//! 实例化 Python 运行时的启动和退出。
pub mod binding;
pub mod call;
pub mod class;
pub mod consts;
pub mod host;
pub mod init;
pub mod limits;
pub mod plugin;
pub mod storage;
pub mod tasks;
use crate::error::PyPluginError;
use colored::Colorize;
use host::PluginHost;
use pyo3::{PyErr, Python, types::PyTracebackMethods};
use std::sync::Arc;

pub async fn init_py(host: &Arc<PluginHost>) -> anyhow::Result<()> {
    class::regist_class();
    init::init_py_vm()?;
    binding::bind(host)?;
    let worker = host.clone();
    tokio::task::spawn_blocking(move || {
        worker.context.runtime.block_on(storage::load_plugins(&worker))
    })
    .await??;
    host.start_scanning();
    tracing::info!("Python 插件初始化完成");
    Ok(())
}
pub async fn post_py(host: &Arc<PluginHost>) -> anyhow::Result<()> {
    tokio::select! {
        result = host.shutdown() => result,
        _ = tokio::signal::ctrl_c() => Err(PyPluginError::PluginNotStopped.into()),
    }
}

pub fn get_py_err_traceback(py_err: &PyErr, py: Option<Python<'_>>) -> String {
    let traceback = match py {
        Some(py) => match py_err.traceback(py) {
            Some(traceback) => traceback.format().unwrap_or_else(|e| format!("{e:?}")),
            None => "none traceback".to_string(),
        },
        None => Python::attach(|py| match py_err.traceback(py) {
            Some(traceback) => traceback.format().unwrap_or_else(|e| format!("{e:?}")),
            None => "none traceback".to_string(),
        }),
    };

    format!("{traceback}{py_err}").red().to_string()
}

#[cfg(test)]
mod tests;
