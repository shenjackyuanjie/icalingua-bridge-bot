pub mod call;
pub mod class;
pub mod config;
pub mod consts;
pub mod init;

use std::ffi::{CStr, CString};
use std::fmt::Display;
use std::path::Path;
use std::sync::OnceLock;
use std::time::SystemTime;
use std::{collections::HashMap, path::PathBuf};

use class::define::PluginDefinePy;
use colored::Colorize;
use pyo3::{
    Bound, Py, PyErr, PyResult, Python,
    exceptions::PyTypeError,
    intern,
    types::{PyAnyMethods, PyModule, PyTracebackMethods},
};
use tracing::{Level, event, span};

use crate::MainStatus;
use crate::error::{PyPluginError, PyPluginInitError};

use consts::{ica_func, sys_func};

#[derive(Debug)]
pub struct PyPluginStorage {

}

#[derive(Debug)]
pub struct PyPlugin {
    /// 加载好的 PyModule
    py_module: Py<PyModule>,
    /// 是否启用
    enabled: bool,
    /// python 侧返回来的定义
    py_define: PluginDefinePy,
}

// #[derive(Debug)]
// pub struct PyStatus {
//     pub files: PyPlugins,
//     pub config: config::PluginConfigFile,
// }

// pub type PyPlugins = HashMap<PathBuf, PyPlugin>;
// pub type RawPyPlugin = (PathBuf, Option<SystemTime>, String);

// #[allow(non_upper_case_globals)]
// pub static mut PyPluginStatus: OnceLock<PyStatus> = OnceLock::new();

// #[allow(static_mut_refs)]
// impl PyStatus {
//     pub fn init() {
//         let config =
//             config::PluginConfigFile::default_init().expect("初始化 Python 插件配置文件失败");
//         let status = PyStatus {
//             files: HashMap::new(),
//             config,
//         };
//         let _ = unsafe { PyPluginStatus.get_or_init(|| status) };
//     }

//     pub fn get() -> &'static PyStatus { unsafe { PyPluginStatus.get().unwrap() } }

//     pub fn get_mut() -> &'static mut PyStatus { unsafe { PyPluginStatus.get_mut().unwrap() } }

//     /// 添加一个插件
//     pub fn add_file(&mut self, path: PathBuf, plugin: PyPlugin) { self.files.insert(path, plugin); }

//     /// 重新加载一个插件
//     pub fn reload_plugin(&mut self, plugin_name: &str) -> bool {
//         let plugin = self.files.iter_mut().find_map(|(_, plugin)| {
//             if plugin.get_id() == plugin_name {
//                 Some(plugin)
//             } else {
//                 None
//             }
//         });
//         if let Some(plugin) = plugin {
//             plugin.reload_from_file()
//         } else {
//             event!(Level::WARN, "没有找到插件: {}", plugin_name);
//             false
//         }
//     }

//     /// 删除一个插件
//     pub fn delete_file(&mut self, path: &PathBuf) -> Option<PyPlugin> { self.files.remove(path) }

//     pub fn get_status(&self, pluging_id: &str) -> Option<bool> {
//         self.files.iter().find_map(|(_, plugin)| {
//             if plugin.get_id() == pluging_id {
//                 return Some(plugin.enabled);
//             }
//             None
//         })
//     }

//     pub fn set_status(&mut self, pluging_id: &str, status: bool) {
//         self.files.iter_mut().for_each(|(_, plugin)| {
//             if plugin.get_id() == pluging_id {
//                 plugin.enabled = status;
//             }
//         });
//     }

//     pub fn verify_file(&self, path: &PathBuf) -> bool {
//         self.files.get(path).is_some_and(|plugin| plugin.verifiy())
//     }

//     pub fn display() -> String {
//         format!(
//             "Python 插件 {{ {} }}",
//             Self::get()
//                 .files
//                 .values()
//                 .map(|v| v.to_string())
//                 .collect::<Vec<String>>()
//                 .join("\n")
//         )
//     }
// }


// #[derive(Debug)]
// pub struct PyPlugin {
//     pub file_path: PathBuf,
//     pub modify_time: Option<SystemTime>,
//     pub py_module: Py<PyModule>,
//     pub enabled: bool,
// }

// impl PyPlugin {
//     pub fn new(path: PathBuf, modify_time: Option<SystemTime>, module: Py<PyModule>) -> Self {
//         PyPlugin {
//             file_path: path.clone(),
//             modify_time,
//             py_module: module,
//             enabled: false,
//         }
//     }

//     /// 从文件创建一个新的
//     pub fn new_from_path(path: &PathBuf) -> Option<Self> {
//         let raw_file = load_py_file(path);
//         match raw_file {
//             Ok(raw_file) => match Self::try_from(raw_file) {
//                 Ok(plugin) => Some(plugin),
//                 Err(e) => {
//                     event!(Level::WARN, "加载 Python 插件文件{:?}: {} 失败", path, e,);
//                     None
//                 }
//             },
//             Err(e) => {
//                 event!(Level::WARN, "加载插件 {:?}: {:?} 失败", path, e);
//                 None
//             }
//         }
//     }

//     /// 从文件更新
//     pub fn reload_from_file(&mut self) -> bool {
//         let raw_file = load_py_file(&self.file_path);
//         match raw_file {
//             Ok(raw_file) => match Self::try_from(raw_file) {
//                 Ok(plugin) => {
//                     self.py_module = plugin.py_module;
//                     self.modify_time = plugin.modify_time;
//                     self.enabled = PyStatus::get().config.get_status(&self.get_id());
//                     event!(Level::INFO, "更新 Python 插件文件 {:?} 完成", self.file_path);
//                     true
//                 }
//                 Err(e) => {
//                     event!(Level::WARN, "更新 Python 插件文件{:?}: {} 失败", self.file_path, e,);
//                     false
//                 }
//             },
//             Err(e) => {
//                 event!(Level::WARN, "更新插件 {:?}: {:?} 失败", self.file_path, e);
//                 false
//             }
//         }
//     }

//     /// 检查文件是否被修改
//     pub fn verifiy(&self) -> bool {
//         match get_change_time(&self.file_path) {
//             None => false,
//             Some(time) => {
//                 if let Some(changed_time) = self.modify_time {
//                     time.eq(&changed_time)
//                 } else {
//                     true
//                 }
//             }
//         }
//     }

//     pub fn get_id(&self) -> String { plugin_path_as_id(&self.file_path) }

//     // pub fn new_from_raw()
// }

// impl Display for PyPlugin {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         write!(f, "{}({:?})-{}", self.get_id(), self.file_path, self.enabled)
//     }
// }

// pub const CONFIG_DATA_NAME: &str = "CONFIG_DATA";

// fn set_str_cfg_default_plugin(
//     module: &Bound<'_, PyModule>,
//     default: String,
//     path: String,
// ) -> PyResult<()> {
//     let base_path = MainStatus::global_config().py().config_path;

//     let mut base_path: PathBuf = PathBuf::from(base_path);

//     if !base_path.exists() {
//         event!(Level::WARN, "python 插件路径不存在, 创建: {:?}", base_path);
//         std::fs::create_dir_all(&base_path)?;
//     }
//     base_path.push(&path);

//     let config_str: String = if base_path.exists() {
//         event!(Level::INFO, "加载 {:?} 的配置文件 {:?} 中", path, base_path);
//         match std::fs::read_to_string(&base_path) {
//             Ok(v) => v,
//             Err(e) => {
//                 event!(Level::WARN, "配置文件 {:?} 读取失败 {}, 创建默认配置", base_path, e);
//                 // 写入默认配置
//                 std::fs::write(&base_path, &default)?;
//                 default
//             }
//         }
//     } else {
//         event!(Level::WARN, "配置文件 {:?} 不存在, 创建默认配置", base_path);
//         // 写入默认配置
//         std::fs::write(base_path, &default)?;
//         default
//     };

//     if let Err(e) = module.setattr(intern!(module.py(), CONFIG_DATA_NAME), &config_str) {
//         event!(Level::WARN, "Python 插件 {:?} 的配置文件信息设置失败:{:?}", path, e);
//         return Err(PyTypeError::new_err(format!(
//             "Python 插件 {path:?} 的配置文件信息设置失败:{e:?}"
//         )));
//     }

//     // 给到 on config
//     if let Ok(attr) = module.getattr(intern!(module.py(), sys_func::ON_CONFIG)) {
//         if !attr.is_callable() {
//             event!(
//                 Level::WARN,
//                 "Python 插件 {:?} 的 {} 函数不是 Callable",
//                 path,
//                 sys_func::ON_CONFIG
//             );
//             return Ok(());
//         }
//         let args = (config_str.as_bytes(),);
//         if let Err(e) = attr.call1(args) {
//             event!(
//                 Level::WARN,
//                 "Python 插件 {:?} 的 {} 函数返回了一个报错 {}",
//                 path,
//                 sys_func::ON_CONFIG,
//                 e
//             );
//         }
//     }

//     Ok(())
// }

// fn set_bytes_cfg_default_plugin(
//     module: &Bound<'_, PyModule>,
//     default: Vec<u8>,
//     path: String,
// ) -> PyResult<()> {
//     let base_path = MainStatus::global_config().py().config_path;

//     let mut base_path: PathBuf = PathBuf::from(base_path);

//     if !base_path.exists() {
//         event!(Level::WARN, "python 插件路径不存在, 创建: {:?}", base_path);
//         std::fs::create_dir_all(&base_path)?;
//     }
//     base_path.push(&path);

//     let config_vec: Vec<u8> = if base_path.exists() {
//         event!(Level::INFO, "加载 {:?} 的配置文件 {:?} 中", path, base_path);
//         match std::fs::read(&base_path) {
//             Ok(v) => v,
//             Err(e) => {
//                 event!(Level::WARN, "配置文件 {:?} 读取失败 {}, 创建默认配置", base_path, e);
//                 // 写入默认配置
//                 std::fs::write(&base_path, &default)?;
//                 default
//             }
//         }
//     } else {
//         event!(Level::WARN, "配置文件 {:?} 不存在, 创建默认配置", base_path);
//         // 写入默认配置
//         std::fs::write(base_path, &default)?;
//         default
//     };

//     match module.setattr(intern!(module.py(), CONFIG_DATA_NAME), &config_vec) {
//         Ok(()) => (),
//         Err(e) => {
//             warn!("Python 插件 {:?} 的配置文件信息设置失败:{:?}", path, e);
//             return Err(PyTypeError::new_err(format!(
//                 "Python 插件 {path:?} 的配置文件信息设置失败:{e:?}"
//             )));
//         }
//     }

//     // 给到 on config
//     if let Ok(attr) = module.getattr(intern!(module.py(), sys_func::ON_CONFIG)) {
//         if !attr.is_callable() {
//             event!(
//                 Level::WARN,
//                 "Python 插件 {:?} 的 {} 函数不是 Callable",
//                 path,
//                 sys_func::ON_CONFIG
//             );
//             return Ok(());
//         }
//         let args = (&config_vec,);
//         if let Err(e) = attr.call1(args) {
//             event!(
//                 Level::WARN,
//                 "Python 插件 {:?} 的 {} 函数返回了一个报错 {}",
//                 path,
//                 sys_func::ON_CONFIG,
//                 e
//             );
//         }
//     }
//     Ok(())
// }

// // 调用 on_load 函数
// fn call_on_load(module: &Bound<'_, PyModule>, path: &Path) -> PyResult<PluginDefinePy> {
//     match call::get_func(module, sys_func::ON_LOAD) {
//         Ok(on_load_func) => match on_load_func.call0() {
//             Err(py_err) => {
//                 let trace = py_err
//                     .traceback(module.py())
//                     .map(|trace| trace.format().unwrap_or("无法格式化堆栈信息".to_string()))
//                     .unwrap_or("无堆栈跟踪信息".to_string());

//                 event!(
//                     Level::WARN,
//                     "Python 插件 {:?} 的 {} 函数返回了一个报错 {}\ntraceback:\n{}",
//                     path,
//                     sys_func::ON_LOAD,
//                     py_err,
//                     trace
//                 );
//                 Err(py_err)
//             }
//             Ok(val) => val.extract::<PluginDefinePy>(),
//         },
//         Err(e) => {
//             if !matches!(e, PyPluginError::FuncNotFound(_, _)) {
//                 event!(
//                     Level::WARN,
//                     "调用 Python 插件 {:?} 的 {} 函数时出现问题 {:?}",
//                     path,
//                     sys_func::ON_LOAD,
//                     e
//                 );
//             }
//             Err(e.into())
//         }
//     }
// }

// impl TryFrom<RawPyPlugin> for PyPlugin {
//     type Error = PyPluginInitError;
//     fn try_from(value: RawPyPlugin) -> Result<Self, Self::Error> {
//         let (path, modify_time, content) = value;
//         let py_module: Py<PyModule> = match py_module_from_code(&content, &path) {
//             Ok(module) => module,
//             Err(e) => {
//                 event!(Level::WARN, "加载 Python 插件: {:?} 失败", e);
//                 return Err(e.into());
//             }
//         };
//         Python::with_gil(|py| {
//             let module = py_module.bind(py);
//             call_on_load(module, &path)?;
//             Ok(PyPlugin::new(path, modify_time, module.clone().unbind()))
//             // 下面这一堆就可以快乐的注释掉了, 反正有 PluginDefine 这一套了
//             // if let Ok(config_func) = call::get_func(module, config_func::REQUIRE_CONFIG) {
//             //     match config_func.call0() {
//             //         Ok(config) => {
//             //             if config.is_instance_of::<PyTuple>() {
//             //                 // let (config, default) = config.extract::<(String, Vec<u8>)>().unwrap();
//             //                 // let (config, default) = config.extract::<(String, String)>().unwrap();
//             //                 if let Ok((config, default)) = config.extract::<(String, String)>() {
//             //                     set_str_cfg_default_plugin(module, default, config)?;
//             //                 } else if let Ok((config, default)) =
//             //                     config.extract::<(String, Vec<u8>)>()
//             //                 {
//             //                     set_bytes_cfg_default_plugin(module, default, config)?;
//             //                 } else {
//             //                     warn!(
//             //                         "加载 Python 插件 {:?} 的配置文件信息时失败:返回的不是 [str, bytes | str]",
//             //                         path
//             //                     );
//             //                     return Err(PyTypeError::new_err(
//             //                         "返回的不是 [str, bytes | str]".to_string(),
//             //                     ));
//             //                 }
//             //                 // 调用 on_load 函数(无参数)
//             //                 call_on_load(module, &path);
//             //                 Ok(PyPlugin::new(path, modify_time, module.clone().unbind()))
//             //             } else if config.is_none() {
//             //                 // 没有配置文件
//             //                 call_on_load(module, &path);
//             //                 Ok(PyPlugin::new(path, modify_time, module.clone().unbind()))
//             //             } else {
//             //                 warn!(
//             //                     "加载 Python 插件 {:?} 的配置文件信息时失败:返回的不是 [str, str]",
//             //                     path
//             //                 );
//             //                 Err(PyTypeError::new_err("返回的不是 [str, str]".to_string()))
//             //             }
//             //         }
//             //         Err(e) => {
//             //             warn!("加载 Python 插件 {:?} 的配置文件信息时失败:{:?}", path, e);
//             //             Err(e)
//             //         }
//             //     }
//             // } else {
//             //     Ok(PyPlugin::new(path, modify_time, module.clone().unbind()))
//             // }
//         })
//     }
// }


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

/// 插件路径转换为 id
pub fn plugin_path_as_id(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("decode-failed")
        .to_string()
}

pub fn get_change_time(path: &Path) -> Option<SystemTime> { path.metadata().ok()?.modified().ok() }

pub fn py_module_from_code(content: &str, path: &Path) -> PyResult<Py<PyModule>> {
    let c_content = CString::new(content).expect("faild to create c string for content");
    let path_str = path.to_str().unwrap_or_default();
    let c_path = CString::new(path_str).expect("faild to create c string for path");
    let file_name = path.file_name().expect("got a none file").to_str().unwrap_or_default();
    let module_name = CString::new(file_name).expect("faild to create c string for file name");
    Python::with_gil(|py| -> PyResult<Py<PyModule>> {
        let module = PyModule::from_code(
            py,
            &c_content,
            &c_path,
            &module_name,
            // !!!! 请注意, 一定要给他一个名字, cpython 会自动把后面的重名模块覆盖掉前面的
        )?;
        Ok(module.unbind())
    })
}

/// 传入文件路径
/// 返回 hash 和 文件内容
pub fn load_py_file(path: &PathBuf) -> std::io::Result<RawPyPlugin> {
    let changed_time = get_change_time(path);
    let content = std::fs::read_to_string(path)?;
    Ok((path.clone(), changed_time, content))
}

/// Python 侧初始化
pub fn init_py() {
    // 从 全局配置中获取 python 插件路径
    let span = span!(Level::INFO, "py init");
    let _enter = span.enter();

    event!(Level::INFO, "开始初始化 python");

    // 注册东西
    class::regist_class();

    let plugin_path = MainStatus::global_config().py().plugin_path.clone();

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
