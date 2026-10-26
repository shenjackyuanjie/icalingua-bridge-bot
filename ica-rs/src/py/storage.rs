//! 插件持久化和生命周期事务；由实例化宿主提供存储与任务归属。
use super::{
    call::{PluginIdentity, current_plugin_identity, with_plugin_context},
    host::PluginHost,
    plugin::{CallbackTable, LifecycleState, PyPlugin},
};
use crate::error::PyPluginInitError;
use colored::Colorize;
use pyo3::{Py, types::PyModule};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

pub const CONFIG_FILE_NAME: &str = "plugins.toml";
const DEFAULT_CONFIG: &str = "# 这个文件由 shenbot 生成，请谨慎修改。\n";
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
fn lifecycle_error(message: impl Into<String>) -> PyPluginInitError {
    PyPluginInitError::Lifecycle(message.into())
}

#[derive(Serialize, Deserialize, Default)]
pub struct PluginStatus {
    pub plugins: HashMap<String, bool>,
}
impl PluginStatus {
    fn path(host: &PluginHost) -> PathBuf {
        PathBuf::from(&host.context.config.py().config_path).join(CONFIG_FILE_NAME)
    }
    fn load(host: &PluginHost) -> Result<Self, PyPluginInitError> {
        let path = Self::path(host);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(PyPluginInitError::ReadPluginCfgFaild)?;
        toml::from_str(&text).map_err(PyPluginInitError::PluginConfigParseError)
    }
    fn save(&self, host: &PluginHost) -> Result<(), PyPluginInitError> {
        let text = toml::to_string_pretty(self).map_err(|e| lifecycle_error(e.to_string()))?;
        std::fs::write(Self::path(host), format!("{DEFAULT_CONFIG}{text}"))
            .map_err(PyPluginInitError::WritePluginDefaultCfgFaild)
    }
}
#[derive(Default)]
pub struct PyPluginStorage {
    pub storage: HashMap<String, PyPlugin>,
}
impl PyPluginStorage {
    pub fn new() -> Self { Self::default() }
    pub fn get_status(&self, id: &str) -> Option<bool> {
        self.storage.get(id).map(PyPlugin::is_enable)
    }
    pub fn display_plugins(&self, color: bool) -> String {
        let enabled = self.storage.values().filter(|p| p.is_enable()).count();
        let entries = self
            .storage
            .values()
            .map(|p| {
                let name = p.id_and_name();
                match (p.is_enable(), color) {
                    (true, true) => name.green().to_string(),
                    (false, true) => name.red().to_string(),
                    (true, false) => name,
                    (false, false) => format!("{name} [禁用]"),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("插件列表 ({enabled} / {}): \n{entries}", self.storage.len())
    }
}
fn reject_self(id: &str) -> Result<(), PyPluginInitError> {
    if current_plugin_identity().is_some_and(|p| p.plugin_id == id) {
        return Err(lifecycle_error("插件不能从自己的 hook 或生命周期函数中变更自身状态"));
    }
    Ok(())
}
async fn lifecycle(
    identity: PluginIdentity,
    module: Arc<Py<PyModule>>,
    load: bool,
) -> Result<CallbackTable, PyPluginInitError> {
    tokio::task::spawn_blocking(move || {
        with_plugin_context(identity, || {
            if load {
                PyPlugin::call_on_load_module(&module)?;
                Ok(PyPlugin::snapshot_callbacks(&module))
            } else {
                PyPlugin::call_on_unload_module(&module)?;
                Ok(CallbackTable::new())
            }
        })
    })
    .await
    .map_err(|e| lifecycle_error(format!("生命周期任务失败: {e}")))?
}
struct DrainedPlugin {
    identity: PluginIdentity,
    module: Arc<Py<PyModule>>,
    path: PathBuf,
    enabled: bool,
}
async fn drain(host: &Arc<PluginHost>, id: &str) -> Result<DrainedPlugin, PyPluginInitError> {
    reject_self(id)?;
    let drained = {
        let mut storage = host.storage.lock().await;
        let plugin = storage
            .storage
            .get_mut(id)
            .ok_or_else(|| lifecycle_error(format!("未找到插件 {id}")))?;
        if plugin.state() == LifecycleState::Draining {
            return Err(lifecycle_error("插件正在变更生命周期"));
        }
        let drained = DrainedPlugin {
            identity: PluginIdentity::new(id, plugin.generation()),
            module: plugin.py_module.clone(),
            path: plugin.plugin_path(),
            enabled: plugin.is_enable(),
        };
        if plugin.state() == LifecycleState::Disabled {
            return Ok(drained);
        }
        plugin.set_state(LifecycleState::Draining);
        host.lifecycle_changed.send_replace(());
        drained
    };
    host.tasks.abort_schedulers(Some(&drained.identity));
    let result = if host.tasks.wait_hooks(&drained.identity, DRAIN_TIMEOUT).await {
        lifecycle(drained.identity.clone(), drained.module.clone(), false)
            .await
            .map(|_| ())
    } else {
        Err(lifecycle_error("等待插件 hook 排空超过 5 秒"))
    };
    // on_unload 中新建的 Scheduler 也属于被卸载代，不得在回退时复活。
    host.tasks.abort_schedulers(Some(&drained.identity));
    if let Err(error) = result {
        let mut storage = host.storage.lock().await;
        if let Some(plugin) = storage.storage.get_mut(id)
            && plugin.generation() == drained.identity.generation
        {
            plugin.set_state(LifecycleState::Active);
            host.lifecycle_changed.send_replace(());
        }
        return Err(error);
    }
    Ok(drained)
}
async fn restore(
    host: &Arc<PluginHost>,
    old: &DrainedPlugin,
    cause: PyPluginInitError,
) -> Result<(), PyPluginInitError> {
    // 旧代保持 Draining，只有 on_load 成功之后才重新接纳普通事件。
    let restored = lifecycle(old.identity.clone(), old.module.clone(), true).await;
    let mut storage = host.storage.lock().await;
    if let Some(plugin) = storage.storage.get_mut(&old.identity.plugin_id) {
        match restored {
            Ok(callbacks) => {
                plugin.callbacks = callbacks;
                plugin.set_enable(old.enabled);
                plugin.set_state(LifecycleState::Active);
                host.lifecycle_changed.send_replace(());
            }
            Err(error) => {
                host.tasks.abort_schedulers(Some(&old.identity));
                plugin.set_enable(false);
                plugin.set_state(LifecycleState::Disabled);
                host.lifecycle_changed.send_replace(());
                return Err(lifecycle_error(format!("{cause}; 恢复旧插件也失败: {error}")));
            }
        }
    }
    Err(cause)
}
async fn set_plugin_status_inner(
    host: &Arc<PluginHost>,
    id: &str,
    enabled: bool,
) -> Result<(), PyPluginInitError> {
    reject_self(id)?;
    if enabled {
        let (identity, module) = {
            let mut storage = host.storage.lock().await;
            let Some(plugin) = storage.storage.get_mut(id) else {
                return Ok(());
            };
            match plugin.state() {
                LifecycleState::Active => return Ok(()),
                LifecycleState::Draining => return Err(lifecycle_error("插件正在变更生命周期")),
                LifecycleState::Disabled => {}
            }
            plugin.set_state(LifecycleState::Draining);
            host.lifecycle_changed.send_replace(());
            (
                PluginIdentity::new(id, plugin.generation().saturating_add(1)),
                plugin.py_module.clone(),
            )
        };
        let loaded = lifecycle(identity.clone(), module, true).await;
        let mut storage = host.storage.lock().await;
        let plugin =
            storage.storage.get_mut(id).ok_or_else(|| lifecycle_error("启用期间插件消失"))?;
        match loaded {
            Ok(callbacks) => {
                plugin.callbacks = callbacks;
                plugin.set_generation(identity.generation);
                plugin.set_enable(true);
                plugin.set_state(LifecycleState::Active);
                host.lifecycle_changed.send_replace(());
                Ok(())
            }
            Err(error) => {
                host.tasks.abort_schedulers(Some(&identity));
                plugin.set_enable(false);
                plugin.set_state(LifecycleState::Disabled);
                host.lifecycle_changed.send_replace(());
                Err(error)
            }
        }
    } else {
        {
            let storage = host.storage.lock().await;
            if storage.storage.get(id).is_none_or(|p| p.state() == LifecycleState::Disabled) {
                return Ok(());
            }
        }
        let drained = drain(host, id).await?;
        let mut storage = host.storage.lock().await;
        if let Some(plugin) = storage.storage.get_mut(id)
            && plugin.generation() == drained.identity.generation
        {
            plugin.set_enable(false);
            plugin.set_state(LifecycleState::Disabled);
            host.lifecycle_changed.send_replace(());
            plugin.callbacks.clear();
        }
        Ok(())
    }
}
async fn reload_plugin_inner(host: &Arc<PluginHost>, id: &str) -> Result<(), PyPluginInitError> {
    let old = drain(host, id).await?;
    if !old.enabled {
        return Err(lifecycle_error("插件已禁用，不能重载"));
    }
    let worker = host.clone();
    let path = old.path.clone();
    let candidate =
        tokio::task::spawn_blocking(move || PyPlugin::new_from_path(&path, &worker.context)).await;
    let mut candidate = match candidate {
        Ok(Ok(candidate)) => candidate,
        Ok(Err(error)) => return restore(host, &old, error).await,
        Err(error) => return restore(host, &old, lifecycle_error(error.to_string())).await,
    };
    if candidate.id() != id {
        retire(candidate.py_module.clone()).await;
        return restore(host, &old, lifecycle_error("候选插件 ID 与原插件不一致")).await;
    }
    let identity = PluginIdentity::new(id, old.identity.generation.saturating_add(1));
    match lifecycle(identity.clone(), candidate.py_module.clone(), true).await {
        Ok(callbacks) => candidate.callbacks = callbacks,
        Err(error) => {
            host.tasks.abort_schedulers(Some(&identity));
            let _ = lifecycle(identity.clone(), candidate.py_module.clone(), false).await;
            host.tasks.abort_schedulers(Some(&identity));
            retire(candidate.py_module.clone()).await;
            return restore(host, &old, error).await;
        }
    }
    candidate.set_generation(identity.generation);
    candidate.set_state(LifecycleState::Active);
    candidate.set_enable(true);
    host.storage.lock().await.storage.insert(id.to_owned(), candidate);
    host.lifecycle_changed.send_replace(());
    retire(old.module).await;
    tracing::info!(plugin_id = %id, generation = identity.generation, "插件重载完成");
    Ok(())
}
async fn remove_plugin_inner(
    host: &Arc<PluginHost>,
    id: &str,
) -> Result<Option<PyPlugin>, PyPluginInitError> {
    if !host.storage.lock().await.storage.contains_key(id) {
        return Ok(None);
    }
    drain(host, id).await?;
    let removed = host.storage.lock().await.storage.remove(id);
    host.limits.forget(id);
    if let Some(plugin) = &removed {
        retire(plugin.py_module.clone()).await;
    }
    Ok(removed)
}
async fn sync_status_from_file_inner(host: &Arc<PluginHost>) -> Result<(), PyPluginInitError> {
    let status = PluginStatus::load(host)?;
    let current = host
        .storage
        .lock()
        .await
        .storage
        .keys()
        .map(|id| (id.clone(), status.plugins.get(id).copied().unwrap_or(true)))
        .collect::<Vec<_>>();
    for (id, enabled) in current {
        set_plugin_status_inner(host, &id, enabled).await?;
    }
    save_status(host).await
}
pub async fn save_status(host: &Arc<PluginHost>) -> Result<(), PyPluginInitError> {
    let mut status = PluginStatus::load(host)?; // 不覆盖损坏的配置。
    {
        let storage = host.storage.lock().await;
        for (id, plugin) in &storage.storage {
            status.plugins.insert(id.clone(), plugin.is_enable());
        }
    }
    status.save(host)
}
pub async fn unload_plugins(host: &Arc<PluginHost>) {
    let ids = host.storage.lock().await.storage.keys().cloned().collect::<Vec<_>>();
    for id in ids {
        match drain(host, &id).await {
            Ok(old) => {
                if let Some(plugin) = host.storage.lock().await.storage.get_mut(&id)
                    && plugin.generation() == old.identity.generation
                {
                    plugin.set_state(LifecycleState::Disabled); // 保留启用标记供下次启动。
                    host.lifecycle_changed.send_replace(());
                }
            }
            Err(error) => tracing::warn!(plugin_id = %id, error = %error, "插件卸载失败"),
        }
    }
}
pub async fn load_plugins(host: &Arc<PluginHost>) -> Result<(), PyPluginInitError> {
    let status = PluginStatus::load(host);
    if let Err(error) = &status {
        tracing::warn!(error = %error, "插件状态文件损坏，保留原文件并使用默认启用状态");
    }
    if !PathBuf::from(&host.context.config.py().plugin_path).exists() {
        tracing::warn!("插件目录不存在，按零插件启动");
        return Ok(());
    }
    if let Err(error) = scan(host, status.as_ref().ok()).await {
        tracing::warn!(error = %error, "初始插件扫描失败，按当前已加载插件继续启动");
        return Ok(());
    }
    if status.is_ok()
        && let Err(error) = save_status(host).await
    {
        tracing::warn!(error = %error, "插件状态文件写回失败");
    }
    Ok(())
}
pub async fn scan_plugins(host: &Arc<PluginHost>) -> Result<(), PyPluginInitError> {
    scan(host, None).await
}
async fn scan(
    host: &Arc<PluginHost>,
    status: Option<&PluginStatus>,
) -> Result<(), PyPluginInitError> {
    let _scan = host.scan_lock.lock().await;
    if !host.is_accepting() {
        return Ok(());
    }
    let folder = PathBuf::from(&host.context.config.py().plugin_path);
    // 完整枚举成功之后才应用删除，避免目录项读取失败导致误卸载。
    let entries = std::fs::read_dir(&folder)
        .map_err(PyPluginInitError::ReadPluginFaild)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(PyPluginInitError::ReadPluginFaild)?;
    let paths = entries
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "py"))
        .collect::<HashSet<_>>();
    for path in &paths {
        if !host.is_accepting() {
            return Ok(());
        }
        let existing = host
            .storage
            .lock()
            .await
            .storage
            .iter()
            .find(|(_, p)| p.plugin_path() == *path)
            .map(|(id, p)| (id.clone(), p.plugin_hash()));
        if let Some((id, hash)) = existing {
            match std::fs::read(path) {
                Ok(content) if blake3::hash(&content) != hash => {
                    if let Err(error) = reload_plugin_inner(host, &id).await {
                        tracing::warn!(plugin_id = %id, error = %error, "插件重载失败");
                    }
                }
                Err(error) => tracing::warn!(error = %error, "插件读取失败，保留旧代"),
                _ => {}
            }
        } else if path.is_file() {
            match PyPlugin::new_from_path(path, &host.context) {
                Ok(mut plugin) => {
                    let id = plugin.id().to_string();
                    let enabled = status.and_then(|s| s.plugins.get(&id)).copied().unwrap_or(true);
                    plugin.set_enable(enabled);
                    {
                        let mut storage = host.storage.lock().await;
                        if storage.storage.contains_key(&id) {
                            tracing::warn!(plugin_id = %id, "跳过重复插件 ID");
                            drop(storage);
                            retire(plugin.py_module.clone()).await;
                            continue;
                        }
                        storage.storage.insert(id.clone(), plugin);
                    }
                    if enabled && let Err(error) = set_plugin_status_inner(host, &id, true).await {
                        tracing::warn!(plugin_id = %id, error = %error, "插件启用失败");
                    }
                    // 扫描到新文件时记录一次，热重载与首次启动都能在默认级别看到。
                    tracing::info!(plugin_id = %id, enabled, "插件加载完成");
                }
                Err(error) => tracing::warn!(error = %error, "插件加载失败"),
            }
        }
    }
    let removed = host
        .storage
        .lock()
        .await
        .storage
        .iter()
        .filter(|(_, p)| !paths.contains(&p.plugin_path()))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in removed {
        if let Err(error) = remove_plugin_inner(host, &id).await {
            tracing::warn!(plugin_id = %id, error = %error, "插件删除排空失败");
        }
    }
    Ok(())
}

async fn retire(module: Arc<Py<PyModule>>) {
    let _ = tokio::task::spawn_blocking(move || PyPlugin::retire_module(&module)).await;
}

pub async fn release_modules(host: &PluginHost) {
    let plugins = std::mem::take(&mut host.storage.lock().await.storage);
    for (_, plugin) in plugins {
        retire(plugin.py_module.clone()).await;
    }
}

// 整个事务在受跟踪的阻塞任务中运行：调用方取消等待不取消一半的重载，
// 文件 I/O 不占用 async worker；Python 调用归属显式传递到执行线程。
async fn control<T, F, Fut>(host: &Arc<PluginHost>, operation: F) -> Result<T, PyPluginInitError>
where
    T: Send + 'static,
    F: FnOnce(Arc<PluginHost>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, PyPluginInitError>>,
{
    let ticket = host.operations.begin().ok_or_else(|| lifecycle_error("插件服务正在关闭"))?;
    let worker = host.clone();
    let origin = current_plugin_identity();
    host.context
        .runtime
        .spawn_blocking(move || {
            let _ticket = ticket;
            let execute = || worker.context.runtime.clone().block_on(operation(worker));
            if let Some(origin) = origin {
                with_plugin_context(origin, execute)
            } else {
                execute()
            }
        })
        .await
        .map_err(|error| lifecycle_error(format!("插件管理事务异常: {error}")))?
}
pub async fn set_plugin_status(
    host: &Arc<PluginHost>,
    id: &str,
    enabled: bool,
) -> Result<(), PyPluginInitError> {
    let id = id.to_owned();
    control(
        host,
        move |host| async move { set_plugin_status_inner(&host, &id, enabled).await },
    )
    .await
}
pub async fn reload_plugin(host: &Arc<PluginHost>, id: &str) -> Result<(), PyPluginInitError> {
    let id = id.to_owned();
    control(host, move |host| async move { reload_plugin_inner(&host, &id).await }).await
}
pub async fn sync_status_from_file(host: &Arc<PluginHost>) -> Result<(), PyPluginInitError> {
    control(host, |host| async move { sync_status_from_file_inner(&host).await }).await
}
pub async fn sync_status_to_file(host: &Arc<PluginHost>) -> Result<(), PyPluginInitError> {
    control(host, |host| async move { save_status(&host).await }).await
}
