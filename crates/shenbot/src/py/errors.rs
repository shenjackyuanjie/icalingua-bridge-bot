//! Python 错误格式化使用插件约定和解释器 traceback。
use super::{consts::sys_func, get_py_err_traceback};
use crate::error::PyPluginInitError;
use std::fmt::{Display, Formatter};

impl Display for PyPluginInitError {
    /// 将当前值写入格式化输出。
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PyPluginInitError::NoOnloadFunc => {
                write!(f, "插件未包含 初始化函数 {}", sys_func::ON_LOAD)
            }
            PyPluginInitError::NoManifest => {
                write!(f, "插件未包含 基本信息 {}", sys_func::MANIFEST)
            }
            PyPluginInitError::ManifestTypeMismatch(value) => {
                write!(f, "插件的 Manifest 信息类型错误, 应为 PluginManifest, 实际为 {value}")
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
                write!(f, "初始化时出现 pyerr: {}", get_py_err_traceback(py_err, None))
            }
            PyPluginInitError::OnloadFailed(py_err) => {
                write!(
                    f,
                    "{} 初始化时出现 pyerr: {}",
                    sys_func::ON_LOAD,
                    get_py_err_traceback(py_err, None)
                )
            }
            PyPluginInitError::OnUnloadFailed(py_err) => {
                write!(
                    f,
                    "{} 卸载时出现 pyerr: {}",
                    sys_func::ON_UNLOAD,
                    get_py_err_traceback(py_err, None)
                )
            }
            PyPluginInitError::Lifecycle(message) => write!(f, "插件生命周期失败: {message}"),
        }
    }
}
