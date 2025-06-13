use std::{collections::HashMap, fmt::Display};

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
    pub name: String,
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
        name,
        version,
        description = None,
        config = None,
        authors = None,
        homepage = None
    ))]
    pub fn new(
        plugin_id: String,
        name: String,
        version: String,
        description: Option<String>,
        config: Option<HashMap<String, crate::py::class::config::ConfigStoragePy>>,
        authors: Option<Vec<String>>,
        homepage: Option<String>,
    ) -> Self {
        Self {
            plugin_id,
            name,
            version,
            description,
            authors: authors.unwrap_or_default(),
            homepage,
            config: config.unwrap_or_default(),
        }
    }

    pub fn __str__(&self) -> String {
        self.to_string()
    }
}

impl Display for PluginDefinePy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PluginDefinePy {{ plugin_id: {}, name: {}, version: {}, description: {}, authors: {:?}, homepage: {}, config: {:?} }}",
            self.plugin_id,
            self.name,
            self.version,
            self.description.as_ref().unwrap_or(&"None".to_string()),
            self.authors,
            self.homepage.as_ref().unwrap_or(&"None".to_string()),
            self.config,
        )
    }
}
