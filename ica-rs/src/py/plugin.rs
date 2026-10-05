//! 单个 Python 插件的清单、代码和生命周期数据。

use std::{
    collections::HashMap,
    ffi::CString,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use pyo3::{
    Py, PyAny, PyResult, Python,
    types::{PyAnyMethods, PyModule},
};
use tracing::{Level, event};

use super::call::TaskType;
use crate::py::{class::manifest::PluginManifestPy, consts::sys_func};
use crate::{context::BotContext, error::PyPluginInitError};
// 仅用于生成解释器内部模块名，不保存 Bot 业务状态。
static MODULE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub type CallbackTable = HashMap<TaskType, Arc<Py<PyAny>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    Active,
    Draining,
    Disabled,
}

#[derive(Debug)]
pub struct PyPlugin {
    pub py_module: Arc<Py<PyModule>>,
    pub callbacks: CallbackTable,
    enabled: bool,
    state: LifecycleState,
    generation: u64,
    manifest: PluginManifestPy,
    hash_result: blake3::Hash,
    plugin_path: PathBuf,
}

impl PyPlugin {
    /// 构造候选插件。这里只读取模块和配置，不执行 `on_load`。
    pub fn new_from_path(path: &Path, context: &BotContext) -> Result<Self, PyPluginInitError> {
        if !path.exists() || !path.is_file() {
            return Err(PyPluginInitError::PluginNotFound);
        }
        let file_content =
            std::fs::read_to_string(path).map_err(PyPluginInitError::ReadPluginFaild)?;
        let file_name = path.file_name().ok_or(PyPluginInitError::PluginNotFound)?;
        let file_name = file_name.to_string_lossy().to_string();
        let file_path = path.to_string_lossy();
        let plugin_module =
            Arc::new(Self::load_module_from_str(&file_content, &file_name, &file_path)?);
        let manifest = Self::get_manifest_from_module(&plugin_module, &file_name)
            .inspect_err(|_| Self::retire_module(&plugin_module))?;
        let hash_result = {
            let mut hasher = blake3::Hasher::new();
            let _ = hasher.write(file_content.as_bytes());
            hasher.finalize()
        };
        let mut plugin = Self {
            py_module: plugin_module,
            callbacks: HashMap::new(),
            enabled: true,
            state: LifecycleState::Disabled,
            generation: 0,
            manifest,
            hash_result,
            plugin_path: path.to_path_buf(),
        };
        plugin
            .init_manifest(context)
            .inspect_err(|_| Self::retire_module(&plugin.py_module))?;
        plugin.set_manifest();
        Ok(plugin)
    }

    pub fn snapshot_callbacks(module: &Py<PyModule>) -> CallbackTable {
        Python::attach(|py| {
            [
                TaskType::IcaNewMessage,
                TaskType::IcaSystemMessage,
                TaskType::IcaDeleteMessage,
                TaskType::IcaJoinRequest,
                TaskType::TailchatNewMessage,
            ]
            .into_iter()
            .filter_map(|kind| match module.getattr(py, kind.py_func_str()) {
                Ok(callback) if callback.bind(py).is_callable() => Some((kind, Arc::new(callback))),
                Ok(_) => {
                    tracing::warn!(hook = kind.py_func_str(), "Python hook 存在但不可调用，已忽略");
                    None
                }
                Err(_) => None,
            })
            .collect()
        })
    }

    pub fn id(&self) -> &str { &self.manifest.plugin_id }
    pub fn name(&self) -> &str { &self.manifest.name }
    pub fn id_and_name(&self) -> String { format!("{}({})", self.id(), self.name()) }
    pub fn is_enable(&self) -> bool { self.enabled }
    pub fn state(&self) -> LifecycleState { self.state }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn set_enable(&mut self, status: bool) { self.enabled = status }
    pub fn set_state(&mut self, state: LifecycleState) { self.state = state }
    pub fn set_generation(&mut self, generation: u64) { self.generation = generation }
    pub fn plugin_path(&self) -> PathBuf { self.plugin_path.clone() }
    pub fn plugin_hash(&self) -> blake3::Hash { self.hash_result }

    fn init_manifest(&mut self, context: &BotContext) -> Result<(), PyPluginInitError> {
        if !self.manifest.need_config_file() {
            event!(Level::DEBUG, "插件 {} 不需要配置文件", self.name());
            return Ok(());
        }
        let mut plugin_config = PathBuf::from(&context.config.py().config_path);
        plugin_config.push(self.manifest.config_file_name());
        if plugin_config.is_dir() {
            return Err(PyPluginInitError::PluginCfgIsDir(
                plugin_config.to_string_lossy().to_string(),
            ));
        }
        if !plugin_config.exists() {
            event!(
                Level::WARN,
                "插件 {} 的配置文件 {} 不存在，将创建默认配置",
                self.name(),
                plugin_config.to_string_lossy()
            );
            std::fs::write(&plugin_config, self.manifest.save_cfg_as_string())
                .map_err(PyPluginInitError::WritePluginDefaultCfgFaild)?;
            self.manifest.init_with_default();
        } else {
            let cfg_str = std::fs::read_to_string(&plugin_config)
                .map_err(PyPluginInitError::ReadPluginCfgFaild)?;
            let toml_value: toml::Table =
                toml::from_str(&cfg_str).map_err(PyPluginInitError::PluginConfigParseError)?;
            self.manifest.init_with_toml(&toml_value);
        }
        Ok(())
    }

    pub(crate) fn call_on_load_module(module: &Py<PyModule>) -> Result<(), PyPluginInitError> {
        Python::attach(|py| {
            let module = module.bind(py);
            if let Ok(func) = module.getattr(sys_func::ON_LOAD) {
                if !func.is_callable() {
                    return Err(PyPluginInitError::NoOnloadFunc);
                }
                func.call0().map_err(PyPluginInitError::OnloadFailed)?;
            }
            Ok(())
        })
    }

    pub(crate) fn call_on_unload_module(module: &Py<PyModule>) -> Result<(), PyPluginInitError> {
        Python::attach(|py| {
            let module = module.bind(py);
            if let Ok(func) = module.getattr(sys_func::ON_UNLOAD)
                && func.is_callable()
            {
                func.call0().map_err(PyPluginInitError::OnUnloadFailed)?;
            }
            Ok(())
        })
    }

    fn set_manifest(&mut self) {
        Python::attach(|py| {
            let _ = self.py_module.setattr(py, sys_func::MANIFEST, self.manifest.clone());
        })
    }

    fn get_manifest_from_module(
        py_module: &Py<PyModule>,
        module_name: &str,
    ) -> Result<PluginManifestPy, PyPluginInitError> {
        Python::attach(|py| {
            let raw_module = py_module.bind(py);
            match raw_module.getattr(sys_func::MANIFEST) {
                Ok(manifest) => manifest.extract::<PluginManifestPy>().map_err(|_| {
                    let wrong_type = manifest.get_type().to_string();
                    event!(
                        Level::ERROR,
                        "插件 {module_name} 的 manifest 类型错误, 为 {wrong_type}"
                    );
                    PyPluginInitError::ManifestTypeMismatch(wrong_type)
                }),
                Err(error) => {
                    event!(Level::ERROR, "插件 {module_name} 的 manifest 不存在 {error}");
                    Err(PyPluginInitError::NoManifest)
                }
            }
        })
    }

    pub fn retire_module(module: &Py<PyModule>) {
        Python::attach(|py| {
            let result = (|| -> PyResult<()> {
                let modules = PyModule::import(py, "sys")?.getattr("modules")?;
                let name = module.getattr(py, "__name__")?;
                if let Ok(registered) = modules.get_item(name.bind(py))
                    && registered.as_ptr() == module.as_ptr()
                {
                    modules.del_item(name.bind(py))?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                tracing::warn!(error = %error, "清理废弃插件模块绑定失败");
            }
        });
    }

    fn load_module_from_str(
        code: &str,
        _module_name: &str,
        plugin_path: &str,
    ) -> Result<Py<PyModule>, PyPluginInitError> {
        let c_content =
            CString::new(code).map_err(|error| PyPluginInitError::Lifecycle(error.to_string()))?;
        // PyImport_ExecCodeModule 会复用同名模块，候选失败会污染旧代字典。
        // 每代使用独立命名空间；提交/删除后显式移除废弃模块绑定。
        let name = format!("_shenbot_plugin_{}", MODULE_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let module_name = CString::new(name.as_str()).expect("内部模块名不含 NUL");
        let plugin_path = CString::new(plugin_path)
            .map_err(|error| PyPluginInitError::Lifecycle(error.to_string()))?;
        Python::attach(|py| -> PyResult<Py<PyModule>> {
            match PyModule::from_code(py, &c_content, &plugin_path, &module_name) {
                Ok(module) => Ok(module.unbind()),
                Err(error) => {
                    if let Ok(modules) =
                        PyModule::import(py, "sys").and_then(|sys| sys.getattr("modules"))
                    {
                        let _ = modules.del_item(&name);
                    }
                    Err(error)
                }
            }
        })
        .map_err(PyPluginInitError::from)
    }
}
