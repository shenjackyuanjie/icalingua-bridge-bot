//! Python 插件发现、状态持久化和异步生命周期协调。

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};

use colored::Colorize;
use pyo3::{Py, Python, types::PyModule};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{Level, event, span};

use crate::{
    MainStatus,
    error::PyPluginInitError,
    py::{
        PY_PLUGIN_STORAGE,
        call::{
            PluginIdentity, abort_schedulers, current_plugin_identity, wait_for_hooks,
            with_plugin_context,
        },
        plugin::{LifecycleState, PyPlugin},
    },
};

pub const CONFIG_FILE_NAME: &str = "plugins.toml";
pub const DEFAULT_CONFIG: &str = r#"# 这个文件是由 shenbot 自动生成的, 请 **谨慎** 修改
# 请不要修改这个文件, 除非你知道你在做什么"#;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
static SCAN_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginStatus {
    pub plugins: HashMap<String, bool>,
}

impl PluginStatus {
    fn path() -> PathBuf {
        PathBuf::from(MainStatus::global_config().py().config_path).join(CONFIG_FILE_NAME)
    }

    pub fn load_from_file() -> Result<Self, PyPluginInitError> {
        let path = Self::path();
        if !path.is_file() {
            return Ok(Self::default());
        }
        let content =
            std::fs::read_to_string(path).map_err(PyPluginInitError::ReadPluginCfgFaild)?;
        toml::from_str(&content).map_err(PyPluginInitError::PluginConfigParseError)
    }

    pub fn save_to_file(&self) -> Result<(), PyPluginInitError> {
        let content = toml::to_string_pretty(self)
            .map_err(|error| PyPluginInitError::Lifecycle(error.to_string()))?;
        std::fs::write(Self::path(), format!("{DEFAULT_CONFIG}\n{content}"))
            .map_err(PyPluginInitError::WritePluginDefaultCfgFaild)
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

    pub fn display_plugins(&self, color: bool) -> String {
        let enabled_count = self.storage.values().filter(|plugin| plugin.is_enable()).count();
        let total_count = self.storage.len();
        let entries = self
            .storage
            .values()
            .map(|plugin| {
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
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("插件列表 ({enabled_count} / {total_count}): \n{entries}")
    }

    pub fn get_status(&self, plugin_id: &str) -> Option<bool> {
        self.storage.get(plugin_id).map(PyPlugin::is_enable)
    }
}

fn self_lifecycle_request(plugin_id: &str) -> bool {
    current_plugin_identity().is_some_and(|identity| identity.plugin_id == plugin_id)
}

fn clone_module(module: &Py<PyModule>) -> Py<PyModule> { Python::attach(|py| module.clone_ref(py)) }

async fn run_lifecycle_hook(
    identity: PluginIdentity,
    module: Py<PyModule>,
    load: bool,
) -> Result<(), PyPluginInitError> {
    tokio::task::spawn_blocking(move || {
        with_plugin_context(identity, || {
            if load {
                PyPlugin::call_on_load_module(&module)
            } else {
                PyPlugin::call_on_unload_module(&module)
            }
        })
    })
    .await
    .map_err(|error| PyPluginInitError::Lifecycle(format!("生命周期任务异常终止: {error}")))?
}

struct DrainedPlugin {
    identity: PluginIdentity,
    module: Py<PyModule>,
    path: PathBuf,
    enabled: bool,
}

async fn drain_plugin(plugin_id: &str) -> Result<DrainedPlugin, PyPluginInitError> {
    if self_lifecycle_request(plugin_id) {
        return Err(PyPluginInitError::Lifecycle(
            "插件不能从自己的 hook 内禁用、删除或重载自身".to_string(),
        ));
    }
    let drained = {
        let mut storage = PY_PLUGIN_STORAGE.lock().await;
        let plugin = storage
            .storage
            .get_mut(plugin_id)
            .ok_or_else(|| PyPluginInitError::Lifecycle(format!("未找到插件 {plugin_id}")))?;
        match plugin.state() {
            LifecycleState::Draining => {
                return Err(PyPluginInitError::Lifecycle(format!(
                    "插件 {plugin_id} 正在执行生命周期变更"
                )));
            }
            LifecycleState::Disabled => {
                return Ok(DrainedPlugin {
                    identity: PluginIdentity::new(plugin_id, plugin.generation()),
                    module: clone_module(&plugin.py_module),
                    path: plugin.plugin_path(),
                    enabled: plugin.is_enable(),
                });
            }
            LifecycleState::Active => plugin.set_state(LifecycleState::Draining),
        }
        DrainedPlugin {
            identity: PluginIdentity::new(plugin_id, plugin.generation()),
            module: clone_module(&plugin.py_module),
            path: plugin.plugin_path(),
            enabled: plugin.is_enable(),
        }
    };

    abort_schedulers(&drained.identity).await;
    if !wait_for_hooks(&drained.identity, DRAIN_TIMEOUT).await {
        let mut storage = PY_PLUGIN_STORAGE.lock().await;
        if let Some(plugin) = storage.storage.get_mut(plugin_id)
            && plugin.generation() == drained.identity.generation
            && plugin.state() == LifecycleState::Draining
        {
            plugin.set_state(LifecycleState::Active);
        }
        return Err(PyPluginInitError::Lifecycle(format!(
            "等待插件 {plugin_id} hook 结束超过 {} 秒",
            DRAIN_TIMEOUT.as_secs()
        )));
    }

    if let Err(error) =
        run_lifecycle_hook(drained.identity.clone(), clone_module(&drained.module), false).await
    {
        let mut storage = PY_PLUGIN_STORAGE.lock().await;
        if let Some(plugin) = storage.storage.get_mut(plugin_id)
            && plugin.generation() == drained.identity.generation
        {
            plugin.set_state(LifecycleState::Active);
        }
        return Err(error);
    }
    Ok(drained)
}

async fn restore_old_plugin(
    plugin_id: &str,
    drained: &DrainedPlugin,
    cause: PyPluginInitError,
) -> Result<(), PyPluginInitError> {
    {
        let mut storage = PY_PLUGIN_STORAGE.lock().await;
        if let Some(plugin) = storage.storage.get_mut(plugin_id) {
            plugin.set_state(LifecycleState::Active);
            plugin.set_enable(drained.enabled);
        }
    }
    match run_lifecycle_hook(drained.identity.clone(), clone_module(&drained.module), true).await {
        Ok(()) => Err(cause),
        Err(restore_error) => {
            abort_schedulers(&drained.identity).await;
            let mut storage = PY_PLUGIN_STORAGE.lock().await;
            if let Some(plugin) = storage.storage.get_mut(plugin_id) {
                plugin.set_state(LifecycleState::Disabled);
                plugin.set_enable(false);
            }
            Err(PyPluginInitError::Lifecycle(format!(
                "{cause}; 恢复旧插件也失败: {restore_error}"
            )))
        }
    }
}

pub async fn set_plugin_status(plugin_id: &str, enabled: bool) -> Result<(), PyPluginInitError> {
    if !PY_PLUGIN_STORAGE.lock().await.storage.contains_key(plugin_id) {
        return Ok(());
    }
    if enabled {
        if self_lifecycle_request(plugin_id) {
            return Err(PyPluginInitError::Lifecycle(
                "插件不能从自己的 hook 内变更自身状态".to_string(),
            ));
        }
        let (identity, module) = {
            let mut storage = PY_PLUGIN_STORAGE.lock().await;
            let plugin = storage
                .storage
                .get_mut(plugin_id)
                .ok_or_else(|| PyPluginInitError::Lifecycle(format!("未找到插件 {plugin_id}")))?;
            if plugin.state() == LifecycleState::Active {
                plugin.set_enable(true);
                return Ok(());
            }
            if plugin.state() == LifecycleState::Draining {
                return Err(PyPluginInitError::Lifecycle(format!(
                    "插件 {plugin_id} 正在执行生命周期变更"
                )));
            }
            plugin.set_enable(true);
            plugin.set_state(LifecycleState::Active);
            (
                PluginIdentity::new(plugin_id, plugin.generation()),
                clone_module(&plugin.py_module),
            )
        };
        if let Err(error) = run_lifecycle_hook(identity.clone(), module, true).await {
            abort_schedulers(&identity).await;
            let mut storage = PY_PLUGIN_STORAGE.lock().await;
            if let Some(plugin) = storage.storage.get_mut(plugin_id) {
                plugin.set_enable(false);
                plugin.set_state(LifecycleState::Disabled);
            }
            return Err(error);
        }
        return Ok(());
    }

    let state = {
        let storage = PY_PLUGIN_STORAGE.lock().await;
        storage.storage.get(plugin_id).map(PyPlugin::state)
    };
    if state == Some(LifecycleState::Disabled) {
        let mut storage = PY_PLUGIN_STORAGE.lock().await;
        if let Some(plugin) = storage.storage.get_mut(plugin_id) {
            plugin.set_enable(false);
        }
        return Ok(());
    }
    let drained = drain_plugin(plugin_id).await?;
    let mut storage = PY_PLUGIN_STORAGE.lock().await;
    if let Some(plugin) = storage.storage.get_mut(plugin_id)
        && plugin.generation() == drained.identity.generation
    {
        plugin.set_enable(false);
        plugin.set_state(LifecycleState::Disabled);
    }
    Ok(())
}

pub async fn reload_plugin(plugin_id: &str) -> Result<(), PyPluginInitError> {
    let drained = drain_plugin(plugin_id).await?;
    if !drained.enabled {
        return Err(PyPluginInitError::Lifecycle(format!("插件 {plugin_id} 已禁用，不能重载")));
    }
    let path = drained.path.clone();
    let candidate_result =
        tokio::task::spawn_blocking(move || PyPlugin::new_from_path(&path)).await;
    let mut candidate = match candidate_result {
        Ok(Ok(candidate)) => candidate,
        Ok(Err(error)) => return restore_old_plugin(plugin_id, &drained, error).await,
        Err(error) => {
            return restore_old_plugin(
                plugin_id,
                &drained,
                PyPluginInitError::Lifecycle(format!("构造候选插件任务失败: {error}")),
            )
            .await;
        }
    };
    if candidate.id() != plugin_id {
        return restore_old_plugin(
            plugin_id,
            &drained,
            PyPluginInitError::Lifecycle(format!(
                "候选插件 ID {} 与原插件 {plugin_id} 不一致",
                candidate.id()
            )),
        )
        .await;
    }

    let new_identity =
        PluginIdentity::new(plugin_id, drained.identity.generation.saturating_add(1));
    let candidate_module = clone_module(&candidate.py_module);
    if let Err(error) =
        run_lifecycle_hook(new_identity.clone(), clone_module(&candidate_module), true).await
    {
        abort_schedulers(&new_identity).await;
        let _ = run_lifecycle_hook(new_identity.clone(), candidate_module, false).await;
        abort_schedulers(&new_identity).await;
        return restore_old_plugin(plugin_id, &drained, error).await;
    }

    candidate.set_enable(true);
    candidate.set_generation(drained.identity.generation.saturating_add(1));
    candidate.set_state(LifecycleState::Active);
    let mut storage = PY_PLUGIN_STORAGE.lock().await;
    let current_matches = storage.storage.get(plugin_id).is_some_and(|plugin| {
        plugin.generation() == drained.identity.generation
            && plugin.state() == LifecycleState::Draining
    });
    if !current_matches {
        drop(storage);
        abort_schedulers(&new_identity).await;
        let _ = run_lifecycle_hook(new_identity.clone(), candidate_module, false).await;
        abort_schedulers(&new_identity).await;
        return restore_old_plugin(
            plugin_id,
            &drained,
            PyPluginInitError::Lifecycle("提交候选插件时旧代状态已变化".to_string()),
        )
        .await;
    }
    storage.storage.insert(plugin_id.to_string(), candidate);
    Ok(())
}

pub async fn remove_plugin(plugin_id: &str) -> Result<Option<PyPlugin>, PyPluginInitError> {
    let exists = PY_PLUGIN_STORAGE.lock().await.storage.contains_key(plugin_id);
    if !exists {
        return Ok(None);
    }
    let state = PY_PLUGIN_STORAGE.lock().await.storage.get(plugin_id).map(PyPlugin::state);
    if state == Some(LifecycleState::Active) {
        let _ = drain_plugin(plugin_id).await?;
    } else if state == Some(LifecycleState::Draining) {
        return Err(PyPluginInitError::Lifecycle(format!("插件 {plugin_id} 正在执行生命周期变更")));
    }
    Ok(PY_PLUGIN_STORAGE.lock().await.storage.remove(plugin_id))
}

pub async fn load_plugins() -> Result<(), PyPluginInitError> {
    let plugin_folder = PathBuf::from(MainStatus::global_config().py().plugin_path.clone());
    let span = span!(Level::INFO, "加载插件");
    let _enter = span.enter();
    let status = match PluginStatus::load_from_file() {
        Ok(status) => Some(status),
        Err(error) => {
            event!(
                Level::WARN,
                "插件状态文件读取失败，使用内存默认启用状态且不覆盖原文件: {error}"
            );
            None
        }
    };

    let entries = match std::fs::read_dir(&plugin_folder) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            event!(Level::WARN, "插件目录 {plugin_folder:?} 不存在，按零插件启动");
            None
        }
        Err(error) => {
            event!(Level::WARN, "读取插件目录 {plugin_folder:?} 失败: {error}");
            None
        }
    };
    let mut loaded_ids = Vec::new();
    if let Some(entries) = entries {
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    event!(Level::WARN, "读取插件目录项失败，已跳过: {error}");
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "py") || !path.is_file() {
                continue;
            }
            match PyPlugin::new_from_path(&path) {
                Ok(mut plugin) => {
                    let id = plugin.id().to_string();
                    let enabled = status
                        .as_ref()
                        .and_then(|status| status.plugins.get(&id))
                        .copied()
                        .unwrap_or(true);
                    plugin.set_enable(enabled);
                    PY_PLUGIN_STORAGE.lock().await.storage.insert(id.clone(), plugin);
                    loaded_ids.push((id, enabled));
                }
                Err(error) => event!(Level::WARN, "插件路径 {path:?} 加载失败: {error}"),
            }
        }
    }

    for (plugin_id, enabled) in loaded_ids {
        if enabled && let Err(error) = set_plugin_status(&plugin_id, true).await {
            event!(Level::WARN, "插件 {plugin_id} 启动失败: {error}");
        }
    }
    if status.is_some()
        && let Err(error) = sync_status_to_file().await
    {
        event!(Level::WARN, "插件状态文件写回失败，继续启动: {error}");
    }
    Ok(())
}

pub async fn sync_status_from_file() -> Result<(), PyPluginInitError> {
    let status = PluginStatus::load_from_file()?;
    let current = {
        let storage = PY_PLUGIN_STORAGE.lock().await;
        storage
            .storage
            .keys()
            .map(|id| (id.clone(), status.plugins.get(id).copied().unwrap_or(true)))
            .collect::<Vec<_>>()
    };
    for (plugin_id, enabled) in current {
        set_plugin_status(&plugin_id, enabled).await?;
    }
    sync_status_to_file().await
}

pub async fn sync_status_to_file() -> Result<(), PyPluginInitError> {
    // 若现有文件损坏，不覆盖它。
    let mut status = PluginStatus::load_from_file()?;
    let storage = PY_PLUGIN_STORAGE.lock().await;
    for (plugin_id, plugin) in &storage.storage {
        status.plugins.insert(plugin_id.clone(), plugin.is_enable());
    }
    drop(storage);
    status.save_to_file()
}

pub async fn unload_plugins() {
    let ids = PY_PLUGIN_STORAGE.lock().await.storage.keys().cloned().collect::<Vec<_>>();
    for plugin_id in ids {
        let drained = match drain_plugin(&plugin_id).await {
            Ok(drained) => drained,
            Err(error) => {
                event!(Level::WARN, "插件 {plugin_id} 卸载失败: {error}");
                continue;
            }
        };
        let mut storage = PY_PLUGIN_STORAGE.lock().await;
        if let Some(plugin) = storage.storage.get_mut(&plugin_id)
            && plugin.generation() == drained.identity.generation
        {
            let (enabled, state) = shutdown_transition(plugin.is_enable());
            plugin.set_enable(enabled);
            plugin.set_state(state);
        }
    }
}

fn shutdown_transition(enabled: bool) -> (bool, LifecycleState) {
    (enabled, LifecycleState::Disabled)
}

pub async fn scan_plugins(plugin_folder: &Path) -> Result<(), PyPluginInitError> {
    let _scan = SCAN_LOCK.lock().await;
    let entries = std::fs::read_dir(plugin_folder).map_err(PyPluginInitError::ReadPluginFaild)?;
    let mut disk_paths = HashSet::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                event!(Level::WARN, "读取插件目录项失败，已跳过: {error}");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "py") {
            continue;
        }
        disk_paths.insert(path.clone());
        if !path.is_file() {
            continue;
        }
        let existing = {
            let storage = PY_PLUGIN_STORAGE.lock().await;
            storage
                .storage
                .iter()
                .find(|(_, plugin)| plugin.plugin_path() == path)
                .map(|(id, plugin)| (id.clone(), plugin.plugin_hash()))
        };
        if let Some((plugin_id, old_hash)) = existing {
            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(error) => {
                    event!(Level::WARN, "读取插件 {path:?} 失败，已跳过: {error}");
                    continue;
                }
            };
            if blake3::hash(content.as_bytes()) != old_hash {
                match reload_plugin(&plugin_id).await {
                    Ok(()) => event!(Level::INFO, "Python 插件 {path:?} 已重新加载"),
                    Err(error) => event!(Level::WARN, "Python 插件 {path:?} 重载失败: {error}"),
                }
            }
        } else {
            match PyPlugin::new_from_path(&path) {
                Ok(plugin) => {
                    let plugin_id = plugin.id().to_string();
                    PY_PLUGIN_STORAGE.lock().await.storage.insert(plugin_id.clone(), plugin);
                    if let Err(error) = set_plugin_status(&plugin_id, true).await {
                        event!(Level::WARN, "新插件 {plugin_id} 启动失败: {error}");
                    }
                }
                Err(error) => event!(Level::WARN, "新插件 {path:?} 加载失败: {error}"),
            }
        }
    }

    let removed = {
        let storage = PY_PLUGIN_STORAGE.lock().await;
        storage
            .storage
            .iter()
            .filter(|(_, plugin)| !disk_paths.contains(&plugin.plugin_path()))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
    };
    for plugin_id in removed {
        match remove_plugin(&plugin_id).await {
            Ok(_) => event!(Level::INFO, "Python 插件 {plugin_id} 已删除"),
            Err(error) => event!(
                Level::WARN,
                "Python 插件 {plugin_id} 删除排空失败，将在后续扫描重试: {error}"
            ),
        }
    }
    Ok(())
}
