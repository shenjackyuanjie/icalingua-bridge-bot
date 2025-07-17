use pyo3::{PyErr, PyTypeInfo};
use std::error::Error;
use std::fmt::{Display, Formatter};

pub type ClientResult<T, E> = Result<T, E>;

#[derive(Debug)]
pub enum IcaError {
    /// Socket IO 链接错误
    SocketIoError(rust_socketio::error::Error),
    /// 登录失败
    LoginFailed(String),
}

#[derive(Debug)]
pub enum TailchatError {
    /// Socket IO 链接错误
    SocketIoError(rust_socketio::error::Error),
    /// reqwest 相关错误
    ReqwestError(reqwest::Error),
    /// 登录失败
    LoginFailed(String),
}

#[derive(Debug)]
pub enum PyPluginError {
    /// 插件内未找到指定函数
    /// 函数名, 模块名
    FuncNotFound(String, String),
    /// 插件内函数获取错误
    /// pyerr, func_name, module_name
    CouldNotGetFunc(pyo3::PyErr, String, String),
    /// 插件内函数不可调用
    FuncNotCallable(String, String),
    /// 插件内函数调用错误
    /// pyerr, func_name, module_name
    FuncCallError(pyo3::PyErr, String, String),
    /// 插件停不下来!
    PluginNotStopped,
}

#[derive(Debug)]
pub enum PyPluginInitError {
    /// 找不到初始化函数
    NoOnloadFunc,
    /// 找不到 manifest 定义
    NoManifest,
    /// manifest 类型错误
    ManifestTypeMismatch(String),
    /// 找不到插件文件
    PluginNotFound,
    /// 插件文件读取错误
    ReadPluginFaild(std::io::Error),
    /// 插件配置文件是文件夹
    PluginCfgIsDir(String),
    /// 插件配置文件读取错误
    ReadPluginCfgFaild(std::io::Error),
    /// 插件配置文件 toml 解析错误
    PluginConfigParseError(toml::de::Error),
    /// 写入插件配置文件默认内容错误
    WritePluginDefaultCfgFaild(std::io::Error),
    /// onload 函数返回了 err
    OnloadFailed(pyo3::PyErr),
    /// 出现了 pyerror
    PyError(pyo3::PyErr),
}

// #[derive(Debug)]
// pub enum PyPluginManifestError {
//     ///
// }

impl From<rust_socketio::Error> for IcaError {
    fn from(e: rust_socketio::Error) -> Self { IcaError::SocketIoError(e) }
}

impl From<rust_socketio::Error> for TailchatError {
    fn from(e: rust_socketio::Error) -> Self { TailchatError::SocketIoError(e) }
}

impl From<reqwest::Error> for TailchatError {
    fn from(e: reqwest::Error) -> Self { TailchatError::ReqwestError(e) }
}

impl From<pyo3::PyErr> for PyPluginInitError {
    fn from(value: PyErr) -> Self { PyPluginInitError::PyError(value) }
}

impl From<std::io::Error> for PyPluginInitError {
    fn from(value: std::io::Error) -> Self { PyPluginInitError::ReadPluginFaild(value) }
}

impl Display for IcaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            IcaError::SocketIoError(e) => write!(f, "Socket IO 链接错误: {e}"),
            IcaError::LoginFailed(e) => write!(f, "登录失败: {e}"),
        }
    }
}

impl Display for TailchatError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TailchatError::SocketIoError(e) => write!(f, "Socket IO 链接错误: {e}"),
            TailchatError::ReqwestError(e) => write!(f, "Reqwest 错误: {e}"),
            TailchatError::LoginFailed(e) => write!(f, "登录失败: {e}"),
        }
    }
}

impl Display for PyPluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PyPluginError::FuncNotFound(name, module) => {
                write!(f, "插件内未找到函数: {name} in {module}")
            }
            PyPluginError::CouldNotGetFunc(py_err, name, module) => {
                write!(f, "插件内函数获取错误: {py_err:#?}|{name} in {module}")
            }
            PyPluginError::FuncNotCallable(name, module) => {
                write!(f, "插件内函数不可调用: {name} in {module}")
            }
            PyPluginError::FuncCallError(py_err, name, module) => {
                write!(f, "插件内函数调用错误: {py_err:#?}|{name} in {module}")
            }
            PyPluginError::PluginNotStopped => {
                write!(f, "插件未停止")
            }
        }
    }
}

impl Display for PyPluginInitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PyPluginInitError::NoOnloadFunc => {
                write!(f, "插件未包含 初始化函数 {}", crate::py::consts::sys_func::ON_LOAD)
            }
            PyPluginInitError::NoManifest => {
                write!(f, "插件未包含 基本信息 {}", crate::py::consts::sys_func::MANIFEST)
            }
            PyPluginInitError::ManifestTypeMismatch(value) => {
                write!(
                    f,
                    "插件的 Manifest 信息类型错误, 应为 {}, 实际为 {value}",
                    crate::py::class::manifest::PluginManifestPy::NAME
                )
            }
            PyPluginInitError::PluginNotFound => {
                write!(f, "插件文件未找到")
            }
            PyPluginInitError::ReadPluginFaild(e) => {
                write!(f, "读取插件文件内容失败: {e}")
            }
            PyPluginInitError::PluginCfgIsDir(path) => {
                write!(f, "插件配置文件路径 '{path}' 是一个目录")
            }
            PyPluginInitError::ReadPluginCfgFaild(e) => {
                write!(f, "读取插件配置文件内容失败: {e}")
            }
            PyPluginInitError::PluginConfigParseError(e) => {
                write!(f, "解析配置文件错误：{e}")
            }
            PyPluginInitError::WritePluginDefaultCfgFaild(e) => {
                write!(f, "写入插件默认配置文件失败: {e}")
            }
            PyPluginInitError::PyError(py_err) => {
                write!(f, "初始化时出现 pyerr: {}", crate::py::get_py_err_traceback(py_err, None))
            }
            PyPluginInitError::OnloadFailed(py_err) => {
                write!(
                    f,
                    "{} 初始化时出现 pyerr: {}",
                    crate::py::consts::sys_func::ON_LOAD,
                    crate::py::get_py_err_traceback(py_err, None)
                )
            }
        }
    }
}

impl Error for IcaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            IcaError::SocketIoError(e) => Some(e),
            IcaError::LoginFailed(_) => None,
        }
    }
}

impl Error for TailchatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            TailchatError::SocketIoError(e) => Some(e),
            TailchatError::ReqwestError(e) => Some(e),
            TailchatError::LoginFailed(_) => None,
        }
    }
}

impl Error for PyPluginError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PyPluginError::FuncNotFound(_, _) => None,
            PyPluginError::CouldNotGetFunc(e, _, _) => Some(e),
            PyPluginError::FuncNotCallable(_, _) => None,
            PyPluginError::FuncCallError(e, _, _) => Some(e),
            PyPluginError::PluginNotStopped => None,
        }
    }
}

impl Error for PyPluginInitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PyPluginInitError::NoOnloadFunc => None,
            PyPluginInitError::NoManifest => None,
            PyPluginInitError::ManifestTypeMismatch(_) => None,
            PyPluginInitError::PluginNotFound => None,
            PyPluginInitError::ReadPluginFaild(e) => Some(e),
            PyPluginInitError::PluginCfgIsDir(_) => None,
            PyPluginInitError::ReadPluginCfgFaild(e) => Some(e),
            PyPluginInitError::PluginConfigParseError(e) => Some(e),
            PyPluginInitError::WritePluginDefaultCfgFaild(e) => Some(e),
            PyPluginInitError::PyError(e) => Some(e),
            PyPluginInitError::OnloadFailed(e) => Some(e),
        }
    }
}

impl From<PyPluginError> for PyErr {
    fn from(value: PyPluginError) -> Self {
        pyo3::exceptions::PySystemError::new_err(value.to_string())
    }
}

impl From<PyPluginInitError> for PyErr {
    fn from(value: PyPluginInitError) -> Self {
        pyo3::exceptions::PySystemError::new_err(value.to_string())
    }
}
