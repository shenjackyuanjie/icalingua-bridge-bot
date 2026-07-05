//! Python 插件清单及元数据的绑定类型。

use std::{collections::HashMap, fmt::Display};

use pyo3::{PyResult, exceptions::PyKeyError, pyclass, pymethods};
use tracing::{Level, event};

use super::config::ConfigStoragePy;

/// 用于定义插件的基本信息
///
#[pyclass(from_py_object)]
#[pyo3(name = "PluginManifest")]
#[derive(Clone, Debug)]
pub struct PluginManifestPy {
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
    pub config: HashMap<String, ConfigStoragePy>,
    /// 是否初始化过
    #[pyo3(get)]
    inited: bool,
}

impl PluginManifestPy {
    /// 返回插件配置文件名。
    pub fn config_file_name(&self) -> String { format!("{}.toml", self.plugin_id) }

    /// 判断插件是否需要独立配置文件。
    pub fn need_config_file(&self) -> bool { !self.config.is_empty() }

    /// 初始化当前 manifest
    ///
    /// 1. 从 toml 读取配置
    /// 2. 暂时还没有别的
    pub fn init_with_toml(&mut self, cfg: &toml::Table) {
        for (key, config_value) in self.config.iter_mut() {
            match cfg.get(key) {
                Some(table) => {
                    if let Some(table) = table.as_table() {
                        config_value.read_toml(table);
                    } else {
                        event!(
                            Level::WARN,
                            "Config {key} is not table, found {}",
                            table.type_str()
                        );
                    }
                }
                None => {
                    event!(Level::WARN, "Config missing key {key}");
                }
            }
        }
        self.inited = true
    }

    /// 使用默认配置初始化 manifest
    pub fn init_with_default(&mut self) {
        let empty = toml::Table::new();
        for cfg in self.config.values_mut() {
            // 用空表初始化, 也就是全部使用默认值
            cfg.read_toml(&empty);
        }
        self.inited = true
    }

    /// 生成需要保存的 toml
    ///
    /// 返回的是 toml 的 table, 如果需要合并配置项可以直接用于合并
    pub fn save_to_toml(&self) -> toml::Table {
        let mut root_table = toml::Table::new();
        for (key, value) in self.config.iter() {
            let value_toml = value.as_toml(false);
            root_table.insert(key.to_string(), toml::Value::Table(value_toml));
        }
        root_table
    }

    /// 生成直接可以用于保存的 str
    ///
    /// 顺手给你前面fmt一些基本信息，方便你使用
    ///
    /// 用于直接保存到文件的时候使用的
    pub fn save_cfg_as_string(&self) -> String {
        use toml::to_string_pretty;
        let toml_table = self.save_to_toml();
        let cfg_str =
            to_string_pretty(&toml::Value::Table(toml_table)).expect("Cannot format config");

        // 在 配置文件的前面加上一些插件相关注释
        format!(
            r#"# plugin {} ({}) config
# plugin version: {}
# plugin authors: {}
# shenbot version: {}
# ica api version: {}
# tailchat api version: {}

{}"#,
            self.name,
            self.plugin_id,
            self.version,
            self.authors.join(", "),
            crate::VERSION,
            crate::ICA_VERSION,
            crate::TAILCHAT_VERSION,
            cfg_str
        )
    }
}

#[pymethods]
impl PluginManifestPy {
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
    /// 创建并初始化对应的数据结构。
    pub fn new(
        plugin_id: String,
        name: String,
        version: String,
        description: Option<String>,
        config: Option<HashMap<String, ConfigStoragePy>>,
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
            inited: false,
        }
    }

    /// 返回机器人主配置。
    pub fn config(&self, key: &str) -> Option<ConfigStoragePy> { self.config.get(key).cloned() }

    /// 返回未经缺失检查的插件配置对象。
    pub fn config_unchecked(&self, key: &str) -> PyResult<ConfigStoragePy> {
        self.config(key)
            .ok_or_else(|| PyKeyError::new_err(format!("配置项 '{key}' 不存在")))
    }

    /// 返回适合 Python 展示的字符串。
    pub fn __str__(&self) -> String { self.to_string() }

    /// 返回序列化后的插件配置文本。
    pub fn config_str(&self) -> String { self.save_cfg_as_string() }
}

impl Display for PluginManifestPy {
    /// 将当前值写入格式化输出。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PluginDefinePy {{ plugin_id: {}, name: {}, version: {}, description: {}, authors: {:?}, homepage: {}, config: {:?} }}",
            self.plugin_id,
            self.name,
            self.version,
            self.description.as_ref().unwrap_or(&"no description".to_string()),
            self.authors,
            self.homepage.as_ref().unwrap_or(&"no homepage".to_string()),
            self.config,
        )
    }
}
