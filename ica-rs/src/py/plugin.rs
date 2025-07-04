use std::{
    ffi::CString,
    io::Write,
    path::{Path, PathBuf},
};

use pyo3::{
    Py, PyResult, Python,
    types::{PyAnyMethods, PyModule},
};
use tracing::{Level, event};

use crate::py::{class::manifest::PluginManifestPy, consts::sys_func};
use crate::{MainStatus, error::PyPluginInitError};

#[derive(Debug)]
pub struct PyPlugin {
    /// 加载好的 PyModule
    pub py_module: Py<PyModule>,
    /// 是否启用
    enabled: bool,
    /// python 侧返回来的定义
    manifest: PluginManifestPy,
    /// 插件文件代码的 hash（为了确定是否修改的）
    hash_result: blake3::Hash,
    /// 插件文件路径
    plugin_path: PathBuf,
}

impl PyPlugin {
    pub fn new_from_path(path: &Path) -> Result<Self, PyPluginInitError> {
        // 检查 path 是否合法
        // 后期可能支持多文件插件
        if !path.exists() || !path.is_file() {
            return Err(PyPluginInitError::PluginNotFound);
        }
        // 读取文件
        let file_content =
            std::fs::read_to_string(path).map_err(PyPluginInitError::ReadPluginFaild)?;
        let file_name = path.file_name().expect("not a file??").to_string_lossy().to_string();
        let file_path = path.to_string_lossy();
        let plugin_module = Self::load_module_from_str(&file_content, &file_name, &file_path)?;
        let manifest = Self::get_manifest_from_module(&plugin_module, &file_name)?;
        let hash_result = {
            let mut hasher = blake3::Hasher::new();
            let _ = hasher.write(file_content.as_bytes());
            hasher.finalize()
        };
        let mut plugin = Self {
            py_module: plugin_module,
            enabled: true, // default enable
            manifest,
            hash_result,
            plugin_path: path.to_path_buf(),
        };
        plugin.init_self()?;
        Ok(plugin)
    }

    pub fn id(&self) -> &str { &self.manifest.plugin_id }

    pub fn name(&self) -> &str { &self.manifest.name }

    pub fn id_and_name(&self) -> String { format!("{}({})", self.id(), self.name()) }

    pub fn version(&self) -> &str { &self.manifest.version }

    pub fn is_enable(&self) -> bool { self.enabled }

    pub fn set_enable(&mut self, status: bool) { self.enabled = status }

    pub fn plugin_path(&self) -> PathBuf { self.plugin_path.clone() }

    pub fn plugin_hash(&self) -> blake3::Hash { self.hash_result }

    /// 初始化 manifest
    fn init_manifest(&mut self) -> Result<(), PyPluginInitError> {
        // 检测是否需要配置文件
        if !self.manifest.need_config_file() {
            event!(Level::DEBUG, "插件 {} 不需要配置文件", self.name());
            return Ok(());
        }
        // 准备配置文件内容
        let cfg_file_name = self.manifest.config_file_name();
        let mut plugin_config = PathBuf::from(MainStatus::global_config().py().config_path);
        plugin_config.push(cfg_file_name);
        if plugin_config.is_dir() {
            let path_str = plugin_config.to_string_lossy().to_string();
            return Err(PyPluginInitError::PluginCfgIsDir(path_str));
        }
        if !plugin_config.exists() {
            // 如果配置文件缺失
            event!(
                Level::WARN,
                "插件 {} 的配置文件 {} 不存在，将创建默认配置",
                self.name(),
                plugin_config.to_string_lossy()
            );
            // 创建配置文件默认内容
            let default_cfg = self.manifest.save_cfg_as_string();
            // 写入默认内容
            std::fs::write(plugin_config, default_cfg)
                .map_err(PyPluginInitError::WritePluginDefaultCfgFaild)?;
            self.manifest.init_with_default();
        } else {
            // 如果配置文件存在
            let cfg_str = std::fs::read_to_string(&plugin_config)
                .map_err(PyPluginInitError::ReadPluginCfgFaild)?;
            let toml_value: toml::Table =
                toml::from_str(&cfg_str).map_err(PyPluginInitError::PluginConfigParseError)?;
            event!(
                Level::DEBUG,
                "插件 {} 已加载配置文件 {}",
                self.name(),
                plugin_config.to_string_lossy()
            );
            self.manifest.init_with_toml(&toml_value);
        }
        Ok(())
    }

    /// 调用函数的 on_load
    fn call_on_load_func(&self) -> Result<(), PyPluginInitError> {
        Python::with_gil(|py| {
            let module = self.py_module.bind(py);
            if let Ok(func) = module.getattr(sys_func::ON_LOAD) {
                if !func.is_callable() {
                    return Err(PyPluginInitError::NoOnloadFunc);
                }
                if let Err(e) = func.call0() {
                    return Err(PyPluginInitError::OnloadFailed(e));
                }
            }
            Ok(())
        })
    }

    pub fn init_self(&mut self) -> Result<(), PyPluginInitError> {
        self.init_manifest()?;
        self.set_manifest();
        self.call_on_load_func()?;
        Ok(())
    }

    fn set_manifest(&mut self) {
        Python::with_gil(|py| {
            let _ = self.py_module.setattr(py, sys_func::MANIFEST, self.manifest.clone());
        })
    }

    pub fn reload_self(&mut self) -> Result<(), PyPluginInitError> {
        // 尝试保存当前配置（如果失败仅记录日志）
        if self.manifest.need_config_file() {
            let cfg_file_name = self.manifest.config_file_name();
            let mut plugin_config = PathBuf::from(MainStatus::global_config().py().config_path);
            plugin_config.push(cfg_file_name);
            if plugin_config.exists() && !plugin_config.is_dir() {
                let cfg_str = self.manifest.save_cfg_as_string();
                if let Err(e) = std::fs::write(&plugin_config, cfg_str) {
                    event!(
                        Level::WARN,
                        "插件 {} 的配置保存失败（路径: {}），错误: {}",
                        self.name(),
                        plugin_config.to_string_lossy(),
                        e
                    );
                } else {
                    event!(
                        Level::DEBUG,
                        "插件 {} 的配置已保存到 {}",
                        self.name(),
                        plugin_config.to_string_lossy()
                    );
                }
            }
        }

        // 检查 path 是否合法
        if !self.plugin_path.exists() || !self.plugin_path.is_file() {
            return Err(PyPluginInitError::PluginNotFound);
        }
        let path = &self.plugin_path;
        let file_content =
            std::fs::read_to_string(path).map_err(PyPluginInitError::ReadPluginFaild)?;
        let file_name = path.file_name().expect("not a file??").to_string_lossy().to_string();
        let file_path = path.to_string_lossy();
        let plugin_module = Self::load_module_from_str(&file_content, &file_name, &file_path)?;
        let manifest = Self::get_manifest_from_module(&plugin_module, &file_name)?;
        self.hash_result = {
            let mut hasher = blake3::Hasher::new();
            let _ = hasher.write(file_content.as_bytes()); // String -> &str -> &[u8]
            hasher.finalize()
        };
        self.py_module = plugin_module;
        self.manifest = manifest;
        self.init_self()?;
        Ok(())
    }

    fn get_manifest_from_module(
        py_module: &Py<PyModule>,
        module_name: &str,
    ) -> Result<PluginManifestPy, PyPluginInitError> {
        Python::with_gil(|py| {
            let raw_module = py_module.bind(py);
            match raw_module.getattr(sys_func::MANIFEST) {
                Ok(manifest) => match manifest.extract::<PluginManifestPy>() {
                    Ok(result) => Ok(result),
                    Err(_) => {
                        let wrong_type = manifest.get_type().to_string();
                        event!(
                            Level::ERROR,
                            "插件 {module_name} 的 manifest 类型错误, 为 {}",
                            wrong_type
                        );
                        Err(PyPluginInitError::ManifestTypeMismatch(wrong_type))
                    }
                },
                Err(e) => {
                    event!(Level::ERROR, "插件 {module_name} 的 manifest 不存在 {e}");
                    Err(PyPluginInitError::NoManifest)
                }
            }
        })
    }

    fn load_module_from_str(
        code: &str,
        module_name: &str,
        plugin_path: &str,
    ) -> Result<Py<PyModule>, PyPluginInitError> {
        let c_content = CString::new(code).expect("faild to create c string for content");
        let module_name =
            CString::new(module_name).expect("faild to create c string for file name");
        let plugin_path = CString::new(plugin_path).expect("faild to create c string for path");
        Python::with_gil(|py| -> PyResult<Py<PyModule>> {
            let module = PyModule::from_code(
                py,
                &c_content,
                &plugin_path,
                &module_name,
                // !!!! 请注意, 一定要给他一个名字, cpython 会自动把后面的重名模块覆盖掉前面的
            )?;
            Ok(module.unbind())
        })
        .map_err(|e| e.into())
    }
}
