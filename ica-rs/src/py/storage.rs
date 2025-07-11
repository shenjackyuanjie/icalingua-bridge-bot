use std::{collections::HashMap, path::PathBuf};

use colored::Colorize;
use serde::{Deserialize, Serialize};
use tracing::{Level, event, span};

use crate::{MainStatus, error::PyPluginInitError, py::plugin::PyPlugin};

pub const CONFIG_FILE_NAME: &str = "plugins.toml";
pub const DEFAULT_CONFIG: &str = r#"# 这个文件是由 shenbot 自动生成的, 请 **谨慎** 修改
# 请不要修改这个文件, 除非你知道你在做什么"#;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginStatus {
    pub plugins: HashMap<String, bool>,
}

impl PluginStatus {
    fn fmt_bool(b: bool) -> String {
        if b {
            "启用".green().to_string()
        } else {
            "禁用".red().to_string()
        }
    }

    /// 将 storage 的状态同步到 配置文件
    pub fn sync_from_storage(&mut self, storage: &PyPluginStorage) {
        // event!(Level::INFO, "同步插件状态");

        storage.storage.iter().for_each(|(name, plugin)| {
            if let Some(v) = self.plugins.get_mut(name) {
                *v = plugin.is_enable()
            } else {
                self.plugins.insert(name.to_string(), plugin.is_enable());
            }
        });
    }

    pub fn sync_to_storage(&mut self, storage: &mut PyPluginStorage) {
        storage.storage.iter_mut().for_each(|(name, plugin)| {
            let old_state = plugin.is_enable();
            if let Some(new_state) = self.plugins.get(name) {
                if old_state != *new_state {
                    event!(
                        Level::INFO,
                        "插件状态: {} {} -> {}",
                        plugin.id_and_name(),
                        Self::fmt_bool(old_state),
                        Self::fmt_bool(*new_state)
                    );
                    plugin.set_enable(*new_state);
                } else {
                    event!(
                        Level::INFO,
                        "插件状态: {} {} (没变)",
                        plugin.id_and_name(),
                        Self::fmt_bool(old_state),
                    );
                }
            } else {
                event!(
                    Level::INFO,
                    "新插件: {} {}",
                    plugin.id_and_name(),
                    Self::fmt_bool(old_state)
                );
                self.plugins.insert(name.to_string(), old_state);
            }
        });
    }

    pub fn save_to_file(&self) {
        use toml::to_string_pretty;
        let mut cfg_path = PathBuf::from(MainStatus::global_config().py().config_path);
        cfg_path.push(CONFIG_FILE_NAME);
        let cfg_str = to_string_pretty(&self).expect("Cannot format config");
        let _ = std::fs::write(cfg_path, format!("{DEFAULT_CONFIG}\n{cfg_str}"));
    }

    /// 从配置文件读取启禁配置
    pub fn load_from_file() -> Self {
        let mut cfg_path = PathBuf::from(MainStatus::global_config().py().config_path);
        cfg_path.push(CONFIG_FILE_NAME);
        if !cfg_path.is_file() {
            return Self {
                plugins: HashMap::new(),
            };
        }
        let content = std::fs::read_to_string(cfg_path).expect("Failed to read config.");
        toml::from_str(&content).expect("加载插件启用状态的 toml 错误")
    }
}

#[derive(Debug)]
pub struct PyPluginStorage {
    pub storage: HashMap<String, PyPlugin>,
}

impl PyPluginStorage {
    pub fn new() -> Self {
        Self {
            storage: HashMap::new(),
        }
    }

    pub fn load_plugins(&mut self) {
        let plugin_folder = PathBuf::from(MainStatus::global_config().py().plugin_path);
        let span = span!(Level::INFO, "加载插件");
        let _enter = span.enter();
        // 目前仅支持 .py 后缀的单文件插件
        // 也许后期会支持多文件插件
        if plugin_folder.is_dir() {
            match plugin_folder.read_dir() {
                Ok(dir) => {
                    for entry in dir {
                        let entry = entry.expect("Failed to get entry");
                        let path = entry.path();
                        if let Some(ext) = path.extension() {
                            if ext != "py" || !path.is_file() {
                                event!(Level::DEBUG, "跳过 {path:?}");
                                continue;
                            }
                            match PyPlugin::new_from_path(&path) {
                                Ok(plugin) => {
                                    event!(Level::INFO, "插件 {} 加载成功", plugin.id_and_name(),);
                                    let id_and_name = plugin.id_and_name();
                                    if let Some(old_plugin) =
                                        self.storage.insert(plugin.id().to_string(), plugin)
                                    {
                                        event!(
                                            Level::INFO,
                                            "插件 {} 替换了老版本的 {}",
                                            id_and_name,
                                            old_plugin.version()
                                        )
                                    }
                                }
                                Err(e) => {
                                    event!(Level::WARN, "插件路径 {path:?} 加载失败: {e}")
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    event!(Level::WARN, "读取插件路径失败 {:?}", e);
                }
            }
        } else {
            event!(Level::ERROR, "插件目录 {plugin_folder:?} 不存在");
        }
        // 同步状态
        let mut status = PluginStatus::load_from_file();
        status.sync_to_storage(self);
        status.save_to_file();
    }

    pub fn sync_status_from_file(&mut self) {
        let mut status = PluginStatus::load_from_file();
        status.sync_to_storage(self);
        status.save_to_file();
    }

    pub fn sync_status_to_file(&self) {
        let mut status = PluginStatus::load_from_file();
        status.sync_from_storage(self);
        status.save_to_file();
    }

    pub fn add_plugin(&mut self, plugin: PyPlugin) {
        let key = plugin.id().to_string();
        self.storage.insert(key, plugin);
    }

    pub fn remove_plugin_by_id(&mut self, plugin_id: &str) -> Option<PyPlugin> {
        self.storage.remove(plugin_id)
    }

    pub fn remove_plugin_by_path(&mut self, plugin_path: &PathBuf) -> Option<PyPlugin> {
        let find = self
            .storage
            .iter()
            .find(|(_, p)| &p.plugin_path() == plugin_path)
            .map(|p| p.0.to_string())?;
        self.remove_plugin_by_id(&find)
    }

    /// 查看插件
    /// 可以查看是否加载
    pub fn display_plugins(&self, color: bool) -> String {
        let enabled_count = self.storage.values().filter(|v| v.is_enable()).count();
        let total_count = self.storage.len();

        let format_display_plugin = |plugin: &PyPlugin| {
            let name = plugin.id_and_name();
            if plugin.is_enable() {
                if color {
                    name.green().to_string()
                } else {
                    name
                }
            } else if color {
                name.red().to_string()
            } else {
                format!("{name} [禁用]")
            }
        };

        format!(
            "插件列表 ({enabled_count} / {total_count}): \n{}",
            self.storage
                .values()
                .map(format_display_plugin)
                .collect::<Vec<String>>()
                .join("\n"),
        )
    }

    pub fn check_and_reload_by_path(&mut self, path: &PathBuf) -> Result<bool, PyPluginInitError> {
        if let Some(plugin) = self.get_plugin_by_path_mut(path) {
            let new_file_content = std::fs::read_to_string(plugin.plugin_path())
                .map_err(PyPluginInitError::ReadPluginFaild)?;
            let new_hash = {
                let mut hasher = blake3::Hasher::new();
                hasher.update(new_file_content.as_bytes());
                hasher.finalize()
            };
            if new_hash != plugin.plugin_hash() {
                plugin.reload_self()?;
                return Ok(true);
            }
            return Ok(false);
        }
        Ok(false)
    }

    pub fn get_plugin_by_path(&self, path: &PathBuf) -> Option<&PyPlugin> {
        self.storage.iter().find(|(_, p)| &p.plugin_path() == path).map(|p| p.1)
    }

    pub fn get_plugin_by_path_mut(&mut self, path: &PathBuf) -> Option<&mut PyPlugin> {
        self.storage.iter_mut().find(|(_, p)| &p.plugin_path() == path).map(|p| p.1)
    }

    pub fn get_status(&self, plugin_id: &str) -> Option<bool> {
        self.storage.get(plugin_id).map(|p| p.is_enable())
    }

    pub fn set_status(&mut self, plugin_id: &str, status: bool) {
        if let Some(plugin) = self.storage.get_mut(plugin_id) {
            plugin.set_enable(status);
        }
    }

    pub fn get_enabled_plugins(&self) -> HashMap<&String, &PyPlugin> {
        self.storage.iter().filter(|(_, p)| p.is_enable()).collect()
    }
    pub fn get_all_plugins(&self) -> HashMap<&String, &PyPlugin> { self.storage.iter().collect() }
}
