//! 外部进程插件配置及启动前的约束。

use std::{collections::HashSet, path::PathBuf};

use serde::Deserialize;

/// 用于此宿主实例的副本目录、默认超时和插件清单。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExtConfig {
    pub shadow_dir: PathBuf,
    pub handshake_timeout_ms: u64,
    pub default_drain_timeout_ms: u64,
    pub plugins: Vec<ExtPluginConfig>,
}

impl Default for ExtConfig {
    fn default() -> Self {
        Self {
            shadow_dir: "./.ext_runtime".into(),
            handshake_timeout_ms: 5_000,
            default_drain_timeout_ms: 30_000,
            plugins: Vec::new(),
        }
    }
}

/// 独立可执行插件。同一份配置中的 ID 在启用、停用和热切之间保持不变。
#[derive(Debug, Clone, Deserialize)]
pub struct ExtPluginConfig {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    pub binary: PathBuf,
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

impl ExtConfig {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        const DAY_MS: u64 = 86_400_000;
        let valid_timeout = |timeout| (1..=DAY_MS).contains(&timeout);
        anyhow::ensure!(!self.shadow_dir.as_os_str().is_empty(), "ext.shadow_dir 不能为空");
        anyhow::ensure!(
            valid_timeout(self.handshake_timeout_ms)
                && valid_timeout(self.default_drain_timeout_ms),
            "ext 的握手与排空超时必须在 1 到 {DAY_MS} 毫秒之间"
        );
        let mut ids = HashSet::new();
        for plugin in &self.plugins {
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
