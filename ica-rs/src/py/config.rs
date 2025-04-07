use std::{path::Path, str::FromStr};

use toml_edit::{value, DocumentMut, Key, Table, TomlError, Value};
use tracing::{event, Level};

use crate::py::PyStatus;
use crate::MainStatus;

/// ```toml
/// # 这个文件是由 shenbot 自动生成的, 请 **谨慎** 修改
/// # 请不要修改这个文件, 除非你知道你在做什么
///
/// [plugins]
/// "xxxxxxx" = false # 被禁用的插件
/// "xxxxxxx" = true # 被启用的插件
/// ```
#[derive(Debug, Clone)]
pub struct PluginConfigFile {
    pub data: DocumentMut,
}

const CONFIG_KEY: &str = "plugins";
pub const CONFIG_FILE_NAME: &str = "plugins.toml";
pub const DEFAULT_CONFIG: &str = r#"
# 这个文件是由 shenbot 自动生成的, 请 **谨慎** 修改
# 请不要修改这个文件, 除非你知道你在做什么
[plugins]
"#;

#[allow(unused)]
impl PluginConfigFile {
    pub fn from_str(data: &str) -> Result<Self, TomlError> {
        let mut data = DocumentMut::from_str(data)?;
        if data.get(CONFIG_KEY).is_none() {
            event!(Level::WARN, "插件配置文件缺少 plugins 字段, 正在初始化");
            data.insert_formatted(
                &Key::from_str(CONFIG_KEY).unwrap(),
                toml_edit::Item::Table(Table::new()),
            );
        }
        Ok(Self { data })
    }

    pub fn default_init() -> anyhow::Result<Self> {
        let config_path = MainStatus::global_config().py().config_path.clone();
        let path = Path::new(&config_path);
        Self::from_config_path(path)
    }

    pub fn reload_from_default(&mut self) -> bool {
        let new_config = Self::default_init();
        if let Err(e) = new_config {
            event!(Level::ERROR, "从配置文件重加载时遇到错误: {}", e);
            return false;
        }
        let new_config = new_config.unwrap();
        self.data = new_config.data;
        true
    }

    pub fn from_config_path(path: &Path) -> anyhow::Result<Self> {
        let config_path = path.join(CONFIG_FILE_NAME);
        if !config_path.exists() {
            event!(Level::WARN, "插件配置文件不存在, 正在创建");
            std::fs::write(&config_path, DEFAULT_CONFIG)?;
            Ok(Self::from_str(DEFAULT_CONFIG)?)
        } else {
            let data = std::fs::read_to_string(&config_path)?;
            Ok(Self::from_str(&data)?)
        }
    }

    fn get_table(&self) -> Option<&Table> {
        self.data.get(CONFIG_KEY).and_then(|item| item.as_table())
    }

    fn get_table_mut(&mut self) -> Option<&mut Table> {
        self.data.get_mut(CONFIG_KEY).and_then(|item| item.as_table_mut())
    }

    /// 获取插件状态
    /// 默认为 true
    pub fn get_status(&self, plugin_id: &str) -> bool {
        if let Some(item) = self.data.get(CONFIG_KEY) {
            if let Some(table) = item.as_table() {
                if let Some(item) = table.get(plugin_id) {
                    if let Some(bool) = item.as_bool() {
                        return bool;
                    }
                }
            }
        }
        true
    }

    /// 删掉一个状态
    pub fn remove_status(&mut self, path: &Path) -> Option<bool> {
        let path_str = path.to_str().unwrap();
        if let Some(table) = self.get_table_mut() {
            if let Some(item) = table.get_mut(path_str) {
                if let Some(bool) = item.as_bool() {
                    table.remove(path_str);
                    return Some(bool);
                } else {
                    table.remove(path_str);
                    return Some(false);
                }
            }
        }
        None
    }

    /// 设置插件状态
    pub fn set_status(&mut self, path: &Path, status: bool) {
        let path_str = path.to_str().unwrap();
        let table = self.data.get_mut(CONFIG_KEY).unwrap().as_table_mut().unwrap();
        if table.contains_key(path_str) {
            match table.get_mut(path_str).unwrap().as_value_mut() {
                Some(value) => *value = Value::from(status),
                None => {
                    table.insert(path_str, value(status));
                }
            }
        } else {
            table.insert(path_str, value(status));
        }
    }

    /// 从默认文件读取状态
    ///
    /// 返回是否成功
    pub fn read_status_from_default(&mut self) -> bool {
        if !self.reload_from_default() {
            return false;
        }
        event!(Level::INFO, "同步插件状态");
        let plugins = PyStatus::get_mut();
        plugins.files.iter_mut().for_each(|(path, status)| {
            let plugin_id = status.get_id();
            let config_status = self.get_status(&plugin_id);
            event!(
                Level::INFO,
                "插件状态: {}({:?}) {} -> {}",
                status.get_id(),
                path,
                status.enabled,
                config_status
            );
            status.enabled = config_status;
        });
        true
    }

    pub fn sync_status_to_config(&mut self) {
        let plugins = PyStatus::get();
        let table = self.data.get_mut(CONFIG_KEY).unwrap().as_table_mut().unwrap();
        table.clear();
        plugins.files.iter().for_each(|(_, status)| {
            table.insert(&status.get_id(), value(status.enabled));
        });
    }

    pub fn write_to_default(&self) -> Result<(), std::io::Error> {
        let config_path = MainStatus::global_config().py().config_path.clone();
        let config_path = Path::new(&config_path);
        self.write_to_file(config_path)
    }

    pub fn write_to_file(&self, path: &Path) -> Result<(), std::io::Error> {
        let config_path = path.join(CONFIG_FILE_NAME);
        std::fs::write(config_path, self.data.to_string())?;
        Ok(())
    }
}
