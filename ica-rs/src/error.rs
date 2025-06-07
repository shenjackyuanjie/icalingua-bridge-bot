use pyo3::PyErr;
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
    /// onload 函数返回了个空
    /// 返回的具体是啥
    InvalidReturnOnload(String),
    /// onload 函数返回了 err
    OnloadFailed(pyo3::PyErr),
    /// require config 时出现错误
    ConfigFaild(pyo3::PyErr),
    /// 出现了 pyerror
    PyError(pyo3::PyErr),
}

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
            PyPluginInitError::InvalidReturnOnload(name) => {
                // 想要直接引用 NAME 还得导入这玩意
                use pyo3::PyTypeInfo;
                write!(
                    f,
                    "插件的初始化函数返回了一个 type: {name} 的东西, 需要一个 {}",
                    crate::py::class::config::ConfigStoragePy::NAME
                )
            }
            PyPluginInitError::PyError(py_err) => {
                write!(f, "初始化时出现 pyerr: {}", crate::py::get_py_err_traceback(py_err))
            }
            PyPluginInitError::OnloadFailed(py_err) => {
                write!(
                    f,
                    "{} 初始化时出现 pyerr: {}",
                    crate::py::consts::sys_func::ON_LOAD,
                    crate::py::get_py_err_traceback(py_err)
                )
            }
            PyPluginInitError::ConfigFaild(py_err) => {
                write!(
                    f,
                    "{} 初始化时出现 pyerr: {}",
                    crate::py::consts::sys_func::ON_CONFIG,
                    crate::py::get_py_err_traceback(py_err)
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
            PyPluginInitError::InvalidReturnOnload(_) => None,
            PyPluginInitError::PyError(e) => Some(e),
            PyPluginInitError::OnloadFailed(e) => Some(e),
            PyPluginInitError::ConfigFaild(e) => Some(e),
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
