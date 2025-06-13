use std::collections::HashMap;

use pyo3::{pyclass, pymethods};

/// 用于定义插件的基本信息
///
#[pyclass]
#[pyo3(name = "PluginDefine")]
#[derive(Clone, Debug)]
pub struct PluginDefinePy {
    /// 插件ID
    #[pyo3(get, set)]
    pub plugin_id: String,
    /// 插件名称
    #[pyo3(get, set)]
    pub plugin_name: String,
    /// 版本号
    #[pyo3(get, set)]
    pub version: String,
    // /// 依赖
    // pub requirements: Vec<String>,
    /// 插件描述
    #[pyo3(get, set)]
    pub description: Option<String>,
    /// 插件作者
    #[pyo3(get, set)]
    pub authors: Vec<String>,
    /// 插件主页
    #[pyo3(get, set)]
    pub homepage: Option<String>,
    /// 配置信息
    pub config: HashMap<String, crate::py::class::config::ConfigStoragePy>,
}

#[pymethods]
impl PluginDefinePy {
    #[new]
    #[pyo3(signature = (
        plugin_id,
        plugin_name,
        version,
        description = None,
        config = None,
        authors = None,
        homepage = None
    ))]
    pub fn new(
        plugin_id: String,
        plugin_name: String,
        version: String,
        description: Option<String>,
        config: Option<HashMap<String, crate::py::class::config::ConfigStoragePy>>,
        authors: Option<Vec<String>>,
        homepage: Option<String>,
    ) -> Self {
        Self {
            plugin_id,
            plugin_name,
            version,
            description,
            authors: authors.unwrap_or_default(),
            homepage,
            config: config.unwrap_or_default(),
        }
    }
}
