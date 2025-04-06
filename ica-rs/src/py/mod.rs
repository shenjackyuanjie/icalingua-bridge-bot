pub mod call;
pub mod class;
pub mod config;

use std::ffi::CString;
use std::fmt::Display;
use std::path::Path;
use std::sync::OnceLock;
use std::time::SystemTime;
use std::{collections::HashMap, path::PathBuf};

use colored::Colorize;
use pyo3::exceptions::PyTypeError;
use pyo3::types::PyTuple;
use pyo3::{intern, prelude::*};
use tracing::{event, span, warn, Level};

use crate::error::PyPluginError;
use crate::MainStatus;

const REQUIRE_CONFIG_FUNC_NAME: &str = "require_config";
const ON_CONFIG_FUNC_NAME: &str = "on_config";

#[derive(Debug, Clone)]
pub struct PyStatus {
    pub files: PyPlugins,
    pub config: config::PluginConfigFile,
}

pub type PyPlugins = HashMap<PathBuf, PyPlugin>;
pub type RawPyPlugin = (PathBuf, Option<SystemTime>, String);

#[allow(non_upper_case_globals)]
static mut PyPluginStatus: OnceLock<PyStatus> = OnceLock::new();

impl PyStatus {
    pub fn init() {
        let config =
            config::PluginConfigFile::default_init().expect("初始化 Python 插件配置文件失败");
        let status = PyStatus {
            files: HashMap::new(),
            config,
        };
        let _ = unsafe { PyPluginStatus.get_or_init(|| status) };
    }

    pub fn get() -> &'static PyStatus { unsafe { PyPluginStatus.get().unwrap() } }

    pub fn get_mut() -> &'static mut PyStatus { unsafe { PyPluginStatus.get_mut().unwrap() } }

    /// 添加一个插件
    pub fn add_file(&mut self, path: PathBuf, plugin: PyPlugin) { self.files.insert(path, plugin); }

    /// 重新加载一个插件
    pub fn reload_plugin(&mut self, plugin_name: &str) -> bool {
        let plugin = self.files.iter_mut().find_map(|(_, plugin)| {
            if plugin.get_id() == plugin_name {
                Some(plugin)
            } else {
                None
            }
        });
        if let Some(plugin) = plugin {
            plugin.reload_from_file()
        } else {
            event!(Level::WARN, "没有找到插件: {}", plugin_name);
            false
        }
    }

    /// 删除一个插件
    pub fn delete_file(&mut self, path: &PathBuf) -> Option<PyPlugin> { self.files.remove(path) }

    pub fn get_status(&self, pluging_id: &str) -> Option<bool> {
        self.files.iter().find_map(|(_, plugin)| {
            if plugin.get_id() == pluging_id {
                return Some(plugin.enabled);
            }
            None
        })
    }

    pub fn set_status(&mut self, pluging_id: &str, status: bool) {
        self.files.iter_mut().for_each(|(_, plugin)| {
            if plugin.get_id() == pluging_id {
                plugin.enabled = status;
            }
        });
    }

    pub fn verify_file(&self, path: &PathBuf) -> bool {
        self.files.get(path).is_some_and(|plugin| plugin.verifiy())
    }

    pub fn display() -> String {
        format!(
            "Python 插件 {{ {} }}",
            Self::get()
                .files
                .values()
                .map(|v| v.to_string())
                .collect::<Vec<String>>()
                .join("\n")
        )
    }
}

pub fn get_py_err_traceback(py_err: &PyErr) -> String {
    Python::with_gil(|py| match py_err.traceback(py) {
        Some(traceback) => match traceback.format() {
            Ok(trace) => trace,
            Err(e) => format!("{:?}", e),
        },
        None => "".to_string(),
    })
    .red()
    .to_string()
}

#[derive(Debug, Clone)]
pub struct PyPlugin {
    pub file_path: PathBuf,
    pub changed_time: Option<SystemTime>,
    pub py_module: Py<PyAny>,
    pub enabled: bool,
}

impl PyPlugin {
    /// 从文件创建一个新的
    pub fn new_from_path(path: &PathBuf) -> Option<Self> {
        let raw_file = load_py_file(path);
        match raw_file {
            Ok(raw_file) => match Self::try_from(raw_file) {
                Ok(plugin) => Some(plugin),
                Err(e) => {
                    warn!(
                        "加载 Python 插件文件{:?}: {:?} 失败\n{}",
                        path,
                        e,
                        get_py_err_traceback(&e)
                    );
                    None
                }
            },
            Err(e) => {
                warn!("加载插件 {:?}: {:?} 失败", path, e);
                None
            }
        }
    }

    /// 从文件更新
    pub fn reload_from_file(&mut self) -> bool {
        let raw_file = load_py_file(&self.file_path);
        match raw_file {
            Ok(raw_file) => match Self::try_from(raw_file) {
                Ok(plugin) => {
                    self.py_module = plugin.py_module;
                    self.changed_time = plugin.changed_time;
                    self.enabled = PyStatus::get().config.get_status(&self.get_id());
                    event!(Level::INFO, "更新 Python 插件文件 {:?} 完成", self.file_path);
                    true
                }
                Err(e) => {
                    warn!(
                        "更新 Python 插件文件{:?}: {:?} 失败\n{}",
                        self.file_path,
                        e,
                        get_py_err_traceback(&e)
                    );
                    false
                }
            },
            Err(e) => {
                warn!("更新插件 {:?}: {:?} 失败", self.file_path, e);
                false
            }
        }
    }

    /// 检查文件是否被修改
    pub fn verifiy(&self) -> bool {
        match get_change_time(&self.file_path) {
            None => false,
            Some(time) => {
                if let Some(changed_time) = self.changed_time {
                    time.eq(&changed_time)
                } else {
                    true
                }
            }
        }
    }

    pub fn get_id(&self) -> String { plugin_path_as_id(&self.file_path) }
}

impl Display for PyPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({:?})-{}", self.get_id(), self.file_path, self.enabled)
    }
}

pub const CONFIG_DATA_NAME: &str = "CONFIG_DATA";

fn set_str_cfg_default_plugin(
    module: &Bound<'_, PyModule>,
    default: String,
    path: String,
) -> PyResult<()> {
    let base_path = MainStatus::global_config().py().config_path;

    let mut base_path: PathBuf = PathBuf::from(base_path);

    if !base_path.exists() {
        event!(Level::WARN, "python 插件路径不存在, 创建: {:?}", base_path);
        std::fs::create_dir_all(&base_path)?;
    }
    base_path.push(&path);

    let config_str: String = if base_path.exists() {
        event!(Level::INFO, "加载 {:?} 的配置文件 {:?} 中", path, base_path);
        match std::fs::read_to_string(&base_path) {
            Ok(v) => v,
            Err(e) => {
                event!(Level::WARN, "配置文件 {:?} 读取失败 {}, 创建默认配置", base_path, e);
                // 写入默认配置
                std::fs::write(&base_path, &default)?;
                default
            }
        }
    } else {
        event!(Level::WARN, "配置文件 {:?} 不存在, 创建默认配置", base_path);
        // 写入默认配置
        std::fs::write(base_path, &default)?;
        default
    };

    if let Err(e) = module.setattr(intern!(module.py(), CONFIG_DATA_NAME), &config_str) {
        event!(Level::WARN, "Python 插件 {:?} 的配置文件信息设置失败:{:?}", path, e);
        return Err(PyTypeError::new_err(format!(
            "Python 插件 {:?} 的配置文件信息设置失败:{:?}",
            path, e
        )));
    }

    // 给到 on config
    if let Ok(attr) = module.getattr(intern!(module.py(), ON_CONFIG_FUNC_NAME)) {
        if !attr.is_callable() {
            event!(
                Level::WARN,
                "Python 插件 {:?} 的 {} 函数不是 Callable",
                path,
                ON_CONFIG_FUNC_NAME
            );
            return Ok(());
        }
        let args = (config_str.as_bytes(),);
        if let Err(e) = attr.call1(args) {
            event!(
                Level::WARN,
                "Python 插件 {:?} 的 {} 函数返回了一个报错 {}",
                path,
                ON_CONFIG_FUNC_NAME,
                e
            );
        }
    }

    Ok(())
}

fn set_bytes_cfg_default_plugin(
    module: &Bound<'_, PyModule>,
    default: Vec<u8>,
    path: String,
) -> PyResult<()> {
    let base_path = MainStatus::global_config().py().config_path;

    let mut base_path: PathBuf = PathBuf::from(base_path);

    if !base_path.exists() {
        event!(Level::WARN, "python 插件路径不存在, 创建: {:?}", base_path);
        std::fs::create_dir_all(&base_path)?;
    }
    base_path.push(&path);

    let config_vec: Vec<u8> = if base_path.exists() {
        event!(Level::INFO, "加载 {:?} 的配置文件 {:?} 中", path, base_path);
        match std::fs::read(&base_path) {
            Ok(v) => v,
            Err(e) => {
                event!(Level::WARN, "配置文件 {:?} 读取失败 {}, 创建默认配置", base_path, e);
                // 写入默认配置
                std::fs::write(&base_path, &default)?;
                default
            }
        }
    } else {
        event!(Level::WARN, "配置文件 {:?} 不存在, 创建默认配置", base_path);
        // 写入默认配置
        std::fs::write(base_path, &default)?;
        default
    };

    match module.setattr(intern!(module.py(), CONFIG_DATA_NAME), &config_vec) {
        Ok(()) => (),
        Err(e) => {
            warn!("Python 插件 {:?} 的配置文件信息设置失败:{:?}", path, e);
            return Err(PyTypeError::new_err(format!(
                "Python 插件 {:?} 的配置文件信息设置失败:{:?}",
                path, e
            )));
        }
    }

    // 给到 on config
    if let Ok(attr) = module.getattr(intern!(module.py(), ON_CONFIG_FUNC_NAME)) {
        if !attr.is_callable() {
            event!(
                Level::WARN,
                "Python 插件 {:?} 的 {} 函数不是 Callable",
                path,
                ON_CONFIG_FUNC_NAME
            );
            return Ok(());
        }
        let args = (&config_vec,);
        if let Err(e) = attr.call1(args) {
            event!(
                Level::WARN,
                "Python 插件 {:?} 的 {} 函数返回了一个报错 {}",
                path,
                ON_CONFIG_FUNC_NAME,
                e
            );
        }
    }
    Ok(())
}

impl TryFrom<RawPyPlugin> for PyPlugin {
    type Error = PyErr;
    fn try_from(value: RawPyPlugin) -> Result<Self, Self::Error> {
        let (path, changed_time, content) = value;
        let py_module: Py<PyModule> = match py_module_from_code(&content, &path) {
            Ok(module) => module,
            Err(e) => {
                warn!("加载 Python 插件: {:?} 失败", e);
                return Err(e);
            }
        };
        Python::with_gil(|py| {
            let module = py_module.bind(py);
            if let Ok(config_func) = call::get_func(module, REQUIRE_CONFIG_FUNC_NAME) {
                match config_func.call0() {
                    Ok(config) => {
                        if config.is_instance_of::<PyTuple>() {
                            // let (config, default) = config.extract::<(String, Vec<u8>)>().unwrap();
                            // let (config, default) = config.extract::<(String, String)>().unwrap();
                            if let Ok((config, default)) = config.extract::<(String, String)>() {
                                set_str_cfg_default_plugin(module, default, config)?;
                            } else if let Ok((config, default)) =
                                config.extract::<(String, Vec<u8>)>()
                            {
                                set_bytes_cfg_default_plugin(module, default, config)?;
                            } else {
                                warn!(
                                    "加载 Python 插件 {:?} 的配置文件信息时失败:返回的不是 [str, bytes | str]",
                                    path
                                );
                                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                                    "返回的不是 [str, bytes | str]".to_string(),
                                ));
                            }
                            Ok(PyPlugin {
                                file_path: path,
                                changed_time,
                                py_module: module.clone().into_any().unbind(),
                                enabled: true,
                            })
                        } else if config.is_none() {
                            // 没有配置文件
                            Ok(PyPlugin {
                                file_path: path,
                                changed_time,
                                py_module: module.clone().into_any().unbind(),
                                enabled: true,
                            })
                        } else {
                            warn!(
                                "加载 Python 插件 {:?} 的配置文件信息时失败:返回的不是 [str, str]",
                                path
                            );
                            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                                "返回的不是 [str, str]".to_string(),
                            ))
                        }
                    }
                    Err(e) => {
                        warn!("加载 Python 插件 {:?} 的配置文件信息时失败:{:?}", path, e);
                        Err(e)
                    }
                }
            } else {
                Ok(PyPlugin {
                    file_path: path,
                    changed_time,
                    py_module: module.clone().into_any().unbind(),
                    enabled: true,
                })
            }
        })
    }
}

/// 插件路径转换为 id
pub fn plugin_path_as_id(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("decode-failed")
        .to_string()
}

pub fn load_py_plugins(path: &PathBuf) {
    let plugins = PyStatus::get_mut();
    if path.exists() {
        event!(Level::INFO, "找到位于 {:?} 的插件", path);
        // 搜索所有的 py 文件 和 文件夹单层下面的 py 文件
        match path.read_dir() {
            Err(e) => {
                event!(Level::WARN, "读取插件路径失败 {:?}", e);
            }
            Ok(dir) => {
                for entry in dir {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    if let Some(ext) = path.extension() {
                        if ext == "py" {
                            if let Some(plugin) = PyPlugin::new_from_path(&path) {
                                plugins.add_file(path, plugin);
                            }
                        }
                    }
                }
            }
        }
    } else {
        event!(Level::WARN, "插件加载目录不存在: {:?}", path);
    }
    plugins.config.read_status_from_default();
    plugins.config.sync_status_to_config();
    event!(
        Level::INFO,
        "python 插件目录: {:?} 加载完成, 加载到 {} 个插件",
        path,
        plugins.files.len()
    );
}

pub fn get_change_time(path: &Path) -> Option<SystemTime> { path.metadata().ok()?.modified().ok() }

pub fn py_module_from_code(content: &str, path: &Path) -> PyResult<Py<PyModule>> {
    Python::with_gil(|py| -> PyResult<Py<PyModule>> {
        let module = PyModule::from_code(
            py,
            CString::new(content).unwrap().as_c_str(),
            CString::new(path.to_string_lossy().as_bytes()).unwrap().as_c_str(),
            CString::new(path.file_name().unwrap().to_string_lossy().as_bytes())
                .unwrap()
                .as_c_str(),
            // !!!! 请注意, 一定要给他一个名字, cpython 会自动把后面的重名模块覆盖掉前面的
        )
        .map(|module| module.unbind());
        module
    })
}

/// 传入文件路径
/// 返回 hash 和 文件内容
pub fn load_py_file(path: &PathBuf) -> std::io::Result<RawPyPlugin> {
    let changed_time = get_change_time(path);
    let content = std::fs::read_to_string(path)?;
    Ok((path.clone(), changed_time, content))
}

fn init_py_with_env_path(path: &str) {
    unsafe {
        #[cfg(target_os = "windows")]
        use std::ffi::OsStr;
        #[cfg(target_os = "windows")]
        use std::os::windows::ffi::OsStrExt;

        let mut config = std::mem::zeroed::<pyo3::ffi::PyConfig>();
        let config_ptr = &mut config as *mut pyo3::ffi::PyConfig;
        // 初始化配置
        // pyo3::ffi::PyConfig_InitIsolatedConfig(config_ptr);
        pyo3::ffi::PyConfig_InitPythonConfig(config_ptr);

        #[cfg(target_os = "linux")]
        let wide_path = path.as_bytes().iter().map(|i| *i as i32).collect::<Vec<i32>>();
        #[cfg(target_os = "windows")]
        let wide_path = OsStr::new(path).encode_wide().chain(Some(0)).collect::<Vec<u16>>();

        // 设置 prefix 和 exec_prefix
        pyo3::ffi::PyConfig_SetString(config_ptr, &mut config.prefix as *mut _, wide_path.as_ptr());
        pyo3::ffi::PyConfig_SetString(
            config_ptr,
            &mut config.exec_prefix as *mut _,
            wide_path.as_ptr(),
        );

        // 使用 Py_InitializeFromConfig 初始化 python
        let status = pyo3::ffi::Py_InitializeFromConfig(&config as *const _);
        pyo3::ffi::PyEval_SaveThread();
        // 清理配置
        pyo3::ffi::PyConfig_Clear(config_ptr);
        match status._type {
            pyo3::ffi::_PyStatus_TYPE::_PyStatus_TYPE_OK => {
                event!(Level::INFO, "根据配置初始化 python 完成");
            }
            pyo3::ffi::_PyStatus_TYPE::_PyStatus_TYPE_EXIT => {
                event!(Level::ERROR, "初始化 python 时发生错误: EXIT");
            }
            pyo3::ffi::_PyStatus_TYPE::_PyStatus_TYPE_ERROR => {
                event!(Level::ERROR, "初始化 python 时发生错误: ERROR");
                pyo3::ffi::Py_ExitStatusException(status);
            }
        }
    }
}

/// Python 侧初始化
pub fn init_py() {
    // 从 全局配置中获取 python 插件路径
    let span = span!(Level::INFO, "py init");
    let _enter = span.enter();

    let plugin_path = MainStatus::global_config().py().plugin_path;

    let cli_args = std::env::args().collect::<Vec<String>>();
    if cli_args.contains(&"-env".to_string()) {
        let env_path = cli_args.iter().find(|&arg| arg != "-env").expect("未找到 -env 参数的值");
        event!(Level::INFO, "找到 -env 参数: {} 正在初始化", env_path);
        // 判断一下是否有 VIRTUAL_ENV 环境变量
        if let Ok(virtual_env) = std::env::var("VIRTUAL_ENV") {
            event!(Level::WARN, "找到 VIRTUAL_ENV 环境变量: {} 将会被 -env 参数覆盖", virtual_env);
        }
        init_py_with_env_path(env_path);
    } else {
        // 根据 VIRTUAL_ENV 环境变量 进行一些处理
        match std::env::var("VIRTUAL_ENV") {
            Ok(virtual_env) => {
                event!(Level::INFO, "找到 VIRTUAL_ENV 环境变量: {} 正在初始化", virtual_env);
                init_py_with_env_path(&virtual_env);
            }
            Err(_) => {
                event!(Level::INFO, "未找到 VIRTUAL_ENV 环境变量, 正常初始化");
                pyo3::prepare_freethreaded_python();
                event!(Level::INFO, "prepare_freethreaded_python 完成");
            }
        }
    }

    PyStatus::init();
    let plugin_path = PathBuf::from(plugin_path);
    load_py_plugins(&plugin_path);
    event!(Level::DEBUG, "python 插件列表: {}", PyStatus::display());

    event!(Level::INFO, "python 初始化完成")
}

pub async fn post_py() -> anyhow::Result<()> {
    let status = PyStatus::get_mut();
    status.config.sync_status_to_config();
    status.config.write_to_default()?;

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
