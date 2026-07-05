//! 机器人、Icalingua、Tailchat 和 Python 插件配置模型。

use std::env;
use std::fs;

use colored::Colorize;
use serde::Deserialize;
use toml::from_str;

use crate::data_struct::{ica, tailchat};

/// Icalingua bot 的配置
#[derive(Debug, Clone, Deserialize)]
pub struct IcaConfig {
    /// icalingua 私钥
    pub private_key: String,
    /// icalingua 服务器地址
    pub host: String,
    /// bot 的 qq
    pub self_id: ica::UserId,
    /// 提醒的房间
    #[serde(default = "default_empty_i64_vec")]
    pub notice_room: Vec<ica::RoomId>,
    /// 是否提醒
    #[serde(default = "default_false")]
    pub notice_start: bool,
    /// 管理员列表
    #[serde(default = "default_empty_i64_vec")]
    pub admin_list: Vec<ica::UserId>,
    /// 过滤列表
    #[serde(default = "default_empty_i64_vec")]
    pub filter_list: Vec<ica::UserId>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TailchatConfig {
    /// 服务器地址
    pub host: String,
    /// 机器人 App ID
    pub app_id: String,
    /// 机器人 App Secret
    pub app_secret: String,
    /// 提醒的房间
    pub notice_room: Vec<(tailchat::GroupId, tailchat::ConverseId)>,
    /// 是否提醒
    #[serde(default = "default_false")]
    pub notice_start: bool,
    /// 管理员列表
    #[serde(default = "default_empty_str_vec")]
    pub admin_list: Vec<tailchat::UserId>,
    /// 过滤列表
    #[serde(default = "default_empty_str_vec")]
    pub filter_list: Vec<tailchat::UserId>,
}

/// 返回默认插件目录。
fn default_plugin_path() -> String { "./plugins".to_string() }
/// 返回默认配置目录。
fn default_config_path() -> String { "./config".to_string() }

#[derive(Debug, Clone, Deserialize)]
pub struct PyConfig {
    /// 插件路径
    #[serde(default = "default_plugin_path")]
    pub plugin_path: String,
    /// 配置文件夹路径
    #[serde(default = "default_config_path")]
    pub config_path: String,
}

/// 返回默认的空整数列表。
fn default_empty_i64_vec() -> Vec<i64> { Vec::new() }
/// 返回默认的空字符串列表。
fn default_empty_str_vec() -> Vec<String> { Vec::new() }
/// 返回布尔默认值 false。
fn default_false() -> bool { false }

/// 主配置
#[derive(Debug, Clone, Deserialize)]
pub struct BotConfig {
    /// 是否启用 icalingua
    #[serde(default = "default_false")]
    pub enable_ica: bool,
    /// Ica 配置
    pub ica: Option<IcaConfig>,

    /// 是否启用 Tailchat
    #[serde(default = "default_false")]
    pub enable_tailchat: bool,
    /// Tailchat 配置
    pub tailchat: Option<TailchatConfig>,

    /// 是否启用 Python 插件
    #[serde(default = "default_false")]
    pub enable_py: bool,
    /// Python 插件配置
    pub py: Option<PyConfig>,
}

impl BotConfig {
    /// 创建并初始化对应的数据结构。
    pub fn new_from_path(config_file_path: String) -> Self {
        // try read config from file
        let config = fs::read_to_string(&config_file_path).expect("Failed to read config file");
        let ret: Self = from_str(&config).unwrap_or_else(|e| {
            panic!("Failed to parse config file {}\ne:{:?}", &config_file_path, e)
        });
        ret
    }
    /// 创建并初始化对应的数据结构。
    pub fn new_from_cli() -> Self {
        // let config_file_path = env::args().nth(1).expect("No config path given");
        // -c <config_file_path>
        let mut config_file_path = "./config.toml".to_string();
        let mut args = env::args();
        while let Some(arg) = args.next() {
            if arg == "-c" {
                config_file_path = args.next().unwrap_or_else(|| {
                    panic!("{}", "No config path given\nUsage: -c <config_file_path>".red())
                });
                break;
            }
        }
        Self::new_from_path(config_file_path)
    }

    /// 检查是否启用 ica
    pub fn check_ica(&self) -> bool { self.enable_ica }

    /// 检查是否启用 Tailchat
    pub fn check_tailchat(&self) -> bool { self.enable_tailchat }

    /// 检查是否启用 Python 插件
    pub fn check_py(&self) -> bool { self.enable_py }

    /// 返回 Icalingua 配置。
    pub fn ica(&self) -> IcaConfig { self.ica.clone().expect("No ica config found") }
    /// 返回 Tailchat 配置。
    pub fn tailchat(&self) -> TailchatConfig {
        self.tailchat.clone().expect("No tailchat config found")
    }
    /// 返回 Python 插件配置。
    pub fn py(&self) -> PyConfig { self.py.clone().expect("No py config found") }
}
