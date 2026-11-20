//! 外部插件 unit 发现、解析与启动前校验。
//!
//! 主配置只保存全局运行参数。与 Python 脚本同目录的 *.plugins.toml 是外部插件清单的唯一来源。

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use serde::Deserialize;

const UNIT_SUFFIX: &str = ".plugins.toml";

/// 此宿主实例共用的运行副本目录和超时。插件定义由插件目录加载。
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtConfig {
    /// 相对路径以宿主工作目录为基准。
    pub shadow_dir: PathBuf,
    pub handshake_timeout_ms: u64,
    pub default_drain_timeout_ms: u64,
}

impl Default for ExtConfig {
    fn default() -> Self {
        Self {
            shadow_dir: "./.ext_runtime".into(),
            handshake_timeout_ms: 5_000,
            default_drain_timeout_ms: 30_000,
        }
    }
}

/// 单个 unit 的运行参数。ID 只由文件名决定，重载时与插件握手 ID 核对。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtPluginConfig {
    #[serde(skip)]
    pub id: String,
    /// 是否在宿主启动时运行；禁用的 unit 仍占用自己的 ID。
    #[serde(default)]
    pub enabled: bool,
    /// 源二进制；运行副本由宿主生成，不应将副本写入 unit。
    pub binary: PathBuf,
    /// 按顺序原样传递，不经 shell。
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub reload_mode: ReloadMode,
    pub drain_timeout_ms: Option<u64>,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default)]
    pub allow_self_reload: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReloadMode {
    #[default]
    Serial,
    Overlap,
}

/// 启动前构建完整的 unit 快照；只取目录直属的普通文件，不递归或跟随文件链接。
/// 游离的备份、Python 脚本和业务配置都不会被当成可执行插件。
pub(crate) fn load_units(directory: &Path) -> anyhow::Result<Vec<ExtPluginConfig>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("无法读取插件目录 {}", directory.display()));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.context("枚举插件目录失败")?;
        if entry.file_name().to_str().is_some_and(|name| name.ends_with(UNIT_SUFFIX))
            && entry.file_type().context("无法检查外部插件 unit 文件类型")?.is_file()
        {
            paths.push(entry.path());
        }
    }
    // 排序让启动和坏文件诊断顺序稳定，避免依赖操作系统的目录枚举顺序。
    paths.sort_unstable();
    paths.iter().map(|path| read_unit(path)).collect()
}

impl ExtConfig {
    /// 同时验证宿主参数与清单；失败时不启动部分插件，避免留下未知的 ID 所属关系。
    pub(crate) fn validate(&self, plugins: &[ExtPluginConfig]) -> anyhow::Result<()> {
        const DAY_MS: u64 = 86_400_000;
        let valid_timeout = |timeout| (1..=DAY_MS).contains(&timeout);
        anyhow::ensure!(!self.shadow_dir.as_os_str().is_empty(), "ext.shadow_dir 不能为空");
        anyhow::ensure!(
            valid_timeout(self.handshake_timeout_ms)
                && valid_timeout(self.default_drain_timeout_ms),
            "ext 的握手与排空超时必须在 1 到 {DAY_MS} 毫秒之间"
        );
        let mut ids = HashSet::new();
        for plugin in plugins {
            anyhow::ensure!(
                (1..=64).contains(&plugin.id.len())
                    && plugin
                        .id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
                "外部插件 ID 必须由 1 到 64 个 ASCII 字母、数字、下划线或连字符组成"
            );
            anyhow::ensure!(ids.insert(&plugin.id), "外部插件 ID 重复：{}", plugin.id);
            anyhow::ensure!(
                !plugin.binary.as_os_str().is_empty(),
                "外部插件 {} 的 binary 不能为空",
                plugin.id
            );
            anyhow::ensure!(
                plugin.working_dir.as_ref().is_none_or(|path| !path.as_os_str().is_empty()),
                "外部插件 {} 的 working_dir 不能为空",
                plugin.id
            );
            anyhow::ensure!(
                plugin.drain_timeout_ms.is_none_or(valid_timeout),
                "外部插件 {} 的排空超时必须在 1 到 {DAY_MS} 毫秒之间",
                plugin.id
            );
        }
        Ok(())
    }
}

fn read_unit(path: &Path) -> anyhow::Result<ExtPluginConfig> {
    let name = path.file_name().and_then(|name| name.to_str()).expect("已按 unit 后缀筛选");
    let id = name.strip_suffix(UNIT_SUFFIX).expect("已按 unit 后缀筛选");
    let source =
        fs::read_to_string(path).with_context(|| format!("无法读取外部插件 unit {name}"))?;
    // 解析器的诊断可能回显参数原值；只记录文件名，不将潜在凭据写入日志。
    let mut plugin: ExtPluginConfig = toml::from_str(&source)
        .map_err(|_| anyhow::anyhow!("外部插件 unit {name} 无效：请检查 TOML 语法和字段"))?;
    plugin.id = id.to_owned();
    let directory = path.parent().expect("扫描的 unit 具有父目录");
    // 源文件与工作目录相对于 unit；args 中的相对路径由子进程按自身 cwd 解析。
    if !plugin.binary.as_os_str().is_empty() && plugin.binary.is_relative() {
        plugin.binary = directory.join(&plugin.binary);
    }
    plugin.working_dir = match plugin.working_dir.take() {
        Some(dir) if !dir.as_os_str().is_empty() && dir.is_relative() => Some(directory.join(dir)),
        Some(dir) => Some(dir),
        None => Some(directory.to_owned()),
    };
    Ok(plugin)
}

#[cfg(test)]
mod tests {
    use crate::config::BotConfig;

    #[test]
    fn rejects_plugin_fields_in_the_host_settings() {
        // 把 unit 字段误写进宿主配置时必须报错，避免宿主启动却没有加载预期插件。
        let misplaced_unit = r#"
            enable_ext = true
            [ext]
            binary = "fixture.exe"
        "#;
        assert!(toml::from_str::<BotConfig>(misplaced_unit).is_err());
    }
}
