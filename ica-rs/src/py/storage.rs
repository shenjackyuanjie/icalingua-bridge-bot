use std::{collections::HashMap, path::PathBuf};

use colored::Colorize;
use serde::{Deserialize, Serialize};
use tracing::{Level, event};

use crate::{MainStatus, py::plugin::PyPlugin};

const CONFIG_KEY: &str = "plugins";
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
                        "插件状态: {} ({name}) {} -> {}",
                        plugin.id(),
                        Self::fmt_bool(old_state),
                        Self::fmt_bool(*new_state)
                    );
                    plugin.set_enable(*new_state);
                } else {
                    event!(
                        Level::INFO,
                        "插件状态: {} ({name}) {} (没变)",
                        plugin.id(),
                        Self::fmt_bool(old_state),
                    );
                }
            } else {
                event!(
                    Level::INFO,
                    "新插件: {} ({name}) {}",
                    plugin.id(),
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
                            }
                            match PyPlugin::new_from_path(&path) {
                                Ok(plugin) => {
                                    event!(
                                        Level::INFO,
                                        "插件 {} ({}) 加载成功",
                                        plugin.id(),
                                        plugin.name()
                                    );
                                    let id = plugin.id().to_string();
                                    let name = plugin.name().to_string();
                                    if let Some(old_plugin) =
                                        self.storage.insert(plugin.id().to_string(), plugin)
                                    {
                                        event!(
                                            Level::INFO,
                                            "插件 {id} ({name}) 替换了老版本的 {}",
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

    /// 查看插件
    /// 可以查看是否加载
    pub fn display_plugins(&self) -> String {
        let enabled_count = self.storage.values().filter(|v| v.is_enable()).count();
        let total_count = self.storage.len();

        let format_display_plugin = |plugin: &PyPlugin| {
            if plugin.is_enable() {
                plugin.name().green().to_string()
            } else {
                plugin.name().red().to_string()
            }
        };

        format!(
            "插件列表 ({enabled_count} / {total_count}): {}",
            self.storage
                .values()
                .map(format_display_plugin)
                .collect::<Vec<String>>()
                .join(", "),
        )
    }
}
