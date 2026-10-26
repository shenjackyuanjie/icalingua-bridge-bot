//! 一次 Bot 运行实例的 Python 插件宿主。
use super::{
    call::{PluginIdentity, TaskType, with_plugin_context},
    limits::HookLimits,
    plugin::LifecycleState,
    storage::{self, PyPluginStorage},
    tasks::TaskTracker,
};
use crate::context::BotContext;
use pyo3::{Py, PyAny};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};

pub struct PluginHost {
    pub context: Arc<BotContext>,
    pub storage: AsyncMutex<PyPluginStorage>,
    pub tasks: Arc<TaskTracker>,
    pub operations: Arc<super::tasks::Operations>,
    pub limits: Arc<HookLimits>,
    pub scan_lock: AsyncMutex<()>,
    pub ica_connection: super::ica_connection::Connection<rust_socketio::asynchronous::Client>,
    pub lifecycle_changed: watch::Sender<()>,
    pub ica_watch_slots: Arc<tokio::sync::Semaphore>,
    accepting: AtomicBool,
    closed: AtomicBool,
    scan_stop: watch::Sender<bool>,
    scanner: Mutex<Option<JoinHandle<()>>>,
}
impl PluginHost {
    pub fn new(context: Arc<BotContext>) -> Arc<Self> {
        let config = context.config.py.clone().unwrap_or_default();
        let (scan_stop, _) = watch::channel(false);
        Arc::new(Self {
            closed: AtomicBool::new(false),
            accepting: AtomicBool::new(context.config.enable_py),
            context,
            storage: AsyncMutex::new(PyPluginStorage::new()),
            tasks: Arc::new(TaskTracker::default()),
            operations: Arc::new(super::tasks::Operations::default()),
            limits: HookLimits::new(config),
            scan_lock: AsyncMutex::new(()),
            ica_connection: Default::default(),
            lifecycle_changed: watch::channel(()).0,
            ica_watch_slots: Arc::new(tokio::sync::Semaphore::new(64)),
            scan_stop,
            scanner: Mutex::new(None),
        })
    }
    pub fn is_closed(&self) -> bool { self.closed.load(Ordering::Acquire) }
    pub fn is_accepting(&self) -> bool { self.accepting.load(Ordering::Acquire) }
    pub fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Release);
        self.lifecycle_changed.send_replace(());
    }
    pub async fn enqueue<F>(self: &Arc<Self>, identity: PluginIdentity, job: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        self.enqueue_with(identity, || job).await
    }
    pub async fn enqueue_with<F, J>(self: &Arc<Self>, identity: PluginIdentity, make_job: F) -> bool
    where
        F: FnOnce() -> J,
        J: FnOnce() + Send + 'static,
    {
        let storage = self.storage.lock().await;
        if !self.is_accepting()
            || !storage.storage.get(&identity.plugin_id).is_some_and(|p| {
                p.state() == LifecycleState::Active && p.generation() == identity.generation
            })
        {
            return false;
        }
        let Some(admission) = self.limits.try_admit(&identity.plugin_id) else {
            return false;
        };
        let ticket = self.tasks.hook(identity.clone());
        // 接纳与登记受同一注册表锁保护；参数拷贝和用户工厂不在锁内运行。
        drop(storage);
        spawn_hook(&self.context.runtime, admission, ticket, identity, make_job());
        true
    }
    pub async fn snapshots(&self, task_type: TaskType) -> Vec<(PluginIdentity, Arc<Py<PyAny>>)> {
        let storage = self.storage.lock().await;
        storage
            .storage
            .iter()
            .filter(|(_, p)| p.state() == LifecycleState::Active)
            .filter_map(|(id, p)| {
                p.callbacks
                    .get(&task_type)
                    .map(|cb| (PluginIdentity::new(id, p.generation()), cb.clone()))
            })
            .collect()
    }
    pub fn start_scanning(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let period = Duration::from_millis(self.context.config.py().reload_interval_ms);
        let mut stop = self.scan_stop.subscribe();
        let task = self.context.runtime.spawn(async move {
            let mut warning_active = false;
            loop {
                if *stop.borrow() {
                    break;
                }
                tokio::select! { _ = stop.changed() => break, _ = tokio::time::sleep(period) => {} }
                let Some(host) = weak.upgrade() else {
                    break;
                };
                let worker = host.clone();
                let result = tokio::task::spawn_blocking(move || {
                    worker.context.runtime.block_on(storage::scan_plugins(&worker))
                })
                .await;
                match result {
                    Ok(Ok(())) => warning_active = false,
                    result => {
                        if !warning_active {
                            tracing::warn!(error = ?result, "插件扫描失败，保留已加载插件");
                        }
                        warning_active = true;
                    }
                }
            }
        });
        *self.scanner.lock().unwrap() = Some(task);
    }
    pub async fn shutdown(self: &Arc<Self>) -> anyhow::Result<()> {
        self.stop_accepting();
        self.operations.close();
        self.scan_stop.send_replace(true);
        let scanner = self.scanner.lock().unwrap().take();
        if let Some(scanner) = scanner {
            let _ = scanner.await;
        }
        self.operations.wait_idle().await;
        // 与持有注册表锁的接纳/计时器登记建立屏障，随后不会漏计在途任务。
        drop(self.storage.lock().await);
        self.tasks.abort_schedulers(None);
        self.tasks.wait_all().await;
        storage::unload_plugins(self).await;
        let worker = self.clone();
        match tokio::task::spawn_blocking(move || {
            worker.context.runtime.block_on(storage::save_status(&worker))
        })
        .await
        {
            Ok(Ok(())) => {}
            result => tracing::warn!(error = ?result, "插件状态写回失败，继续排空"),
        }
        self.tasks.wait_all().await;
        self.closed.store(true, Ordering::Release);
        storage::release_modules(self).await;
        Ok(())
    }
}

fn spawn_hook<F>(
    runtime: &tokio::runtime::Handle,
    admission: super::limits::Admission,
    ticket: super::tasks::TaskTicket,
    identity: PluginIdentity,
    job: F,
) -> JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    runtime.spawn(async move {
        let execution = admission.execute().await;
        let _ = tokio::task::spawn_blocking(move || {
            let (_ticket, _execution) = (ticket, execution);
            with_plugin_context(identity, job);
        })
        .await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborting_wrapper_does_not_release_running_hook_capacity_or_drain_ticket() {
        let limits = HookLimits::new(crate::config::PyConfig {
            max_concurrent_hooks: 1,
            max_concurrent_hooks_per_plugin: 1,
            max_inflight_hooks: 1,
            max_inflight_hooks_per_plugin: 1,
            ..Default::default()
        });
        let tasks = Arc::new(TaskTracker::default());
        let id = PluginIdentity::new("fixture", 1);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let wrapper = spawn_hook(
            &tokio::runtime::Handle::current(),
            limits.try_admit("fixture").unwrap(),
            tasks.hook(id.clone()),
            id.clone(),
            move || {
                let _ = started_tx.send(());
                let _ = release_rx.recv();
            },
        );
        tokio::time::timeout(Duration::from_secs(2), started_rx).await.unwrap().unwrap();
        wrapper.abort();
        assert!(wrapper.await.unwrap_err().is_cancelled());
        assert!(limits.try_admit("fixture").is_none());
        assert!(!tasks.wait_hooks(&id, Duration::from_millis(10)).await);
        release_tx.send(()).unwrap();
        assert!(tasks.wait_hooks(&id, Duration::from_secs(2)).await);
        // 完成通知之后执行凭证也必须已经归还。
        tokio::time::timeout(Duration::from_secs(2), async {
            while limits.try_admit("fixture").is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
