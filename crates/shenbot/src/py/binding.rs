//! Python 私有模块绑定；核心不读取解释器全局状态。
use super::host::PluginHost;
use crate::context::BotContext;
use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyModule};
use std::sync::{Arc, Weak};

#[derive(Clone)]
#[pyclass(frozen, skip_from_py_object)]
pub struct RuntimeBinding {
    pub context: Arc<BotContext>,
    host: Weak<PluginHost>,
}
impl RuntimeBinding {
    pub fn new(host: &Arc<PluginHost>) -> Self {
        Self {
            context: host.context.clone(),
            host: Arc::downgrade(host),
        }
    }
    pub fn from_module(py: Python<'_>) -> PyResult<Self> {
        let module = PyModule::import(py, "shenbot_api")?;
        let binding = module
            .getattr("_runtime_context")
            .map_err(|_| PyRuntimeError::new_err("Bot 上下文尚未绑定"))?;
        Ok(binding.extract::<PyRef<'_, Self>>()?.clone())
    }
    pub fn with_host<T: Send>(
        &self,
        py: Python<'_>,
        call: impl FnOnce(&Arc<PluginHost>) -> T + Send,
    ) -> PyResult<T> {
        let host = self
            .host
            .upgrade()
            .filter(|host| host.is_accepting() && !host.is_closed())
            .ok_or_else(|| PyRuntimeError::new_err("Bot 插件服务正在关闭或已经停止"))?;
        Ok(py.detach(move || call(&host)))
    }
}
pub fn bind(host: &Arc<PluginHost>) -> PyResult<()> {
    Python::attach(|py| {
        PyModule::import(py, "shenbot_api")?.setattr("_runtime_context", RuntimeBinding::new(host))
    })
}
