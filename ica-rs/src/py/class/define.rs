use std::collections::HashMap;

use pyo3::{pyclass, pymethods};

/// 用于定义插件的基本信息
///
#[pyclass]
#[pyo3(name = "PluginDefine")]
pub struct PluginDefinePy {
    /// 插件ID
    pub id: String,
    /// 版本号
    pub version: String,
    /// 依赖
    pub requirements: Vec<String>,
    /// 插件描述
    pub description: Option<String>,
    /// 插件作者
    pub authors: Vec<String>,
    /// 插件主页
    pub homepage: Option<String>,
    // 配置信息
    // pub config: HashMap<String, Config>
    // 权限信息
    // (如果你需要整个权限啥的)
    // pub permissions: HashMap<String, Permission>,
}
