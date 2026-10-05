//! hook 接纳与执行分别限流；没有隐藏的无界阻塞任务队列。
use crate::config::PyConfig;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct OverloadReport {
    last_warning: Instant,
    interval_rejected: usize,
    total_rejected: usize,
}
struct Quota {
    inflight: Arc<Semaphore>,
    running: Arc<Semaphore>,
    warning: Mutex<Option<OverloadReport>>,
}
pub struct HookLimits {
    inflight: Arc<Semaphore>,
    running: Arc<Semaphore>,
    plugins: Mutex<HashMap<String, Arc<Quota>>>,
    config: PyConfig,
    pub rejected: AtomicUsize,
    peak: AtomicUsize,
}
pub struct Admission {
    owner: Arc<HookLimits>,
    quota: Arc<Quota>,
    plugin_id: String,
    queued_at: Instant,
    _global: OwnedSemaphorePermit,
    _plugin: OwnedSemaphorePermit,
}
pub struct Execution {
    _admission: Admission,
    _global: OwnedSemaphorePermit,
    _plugin: OwnedSemaphorePermit,
    started: Instant,
}
impl HookLimits {
    pub fn new(config: PyConfig) -> Arc<Self> {
        Arc::new(Self {
            inflight: Arc::new(Semaphore::new(config.max_inflight_hooks)),
            running: Arc::new(Semaphore::new(config.max_concurrent_hooks)),
            plugins: Mutex::new(HashMap::new()),
            config,
            rejected: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        })
    }
    pub fn try_admit(self: &Arc<Self>, plugin_id: &str) -> Option<Admission> {
        let quota = self
            .plugins
            .lock()
            .unwrap()
            .entry(plugin_id.to_owned())
            .or_insert_with(|| {
                Arc::new(Quota {
                    inflight: Arc::new(Semaphore::new(self.config.max_inflight_hooks_per_plugin)),
                    running: Arc::new(Semaphore::new(self.config.max_concurrent_hooks_per_plugin)),
                    warning: Mutex::new(None),
                })
            })
            .clone();
        let permits = quota.inflight.clone().try_acquire_owned().ok().and_then(|plugin| {
            self.inflight.clone().try_acquire_owned().ok().map(|global| (plugin, global))
        });
        let Some((plugin, global)) = permits else {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            let mut warning = quota.warning.lock().unwrap();
            let now = Instant::now();
            match warning.as_mut() {
                None => {
                    tracing::warn!(plugin_id, rejected = 1, "Python hook 过载，本次投递已拒绝");
                    *warning = Some(OverloadReport {
                        last_warning: now,
                        interval_rejected: 0,
                        total_rejected: 1,
                    });
                }
                Some(report) => {
                    report.interval_rejected += 1;
                    report.total_rejected += 1;
                    if report.last_warning.elapsed() >= Duration::from_secs(30) {
                        tracing::warn!(
                            plugin_id,
                            rejected = report.interval_rejected,
                            "Python hook 持续过载，继续拒绝新投递"
                        );
                        report.last_warning = now;
                        report.interval_rejected = 0;
                    }
                }
            }
            return None;
        };
        if let Some(report) = quota.warning.lock().unwrap().take() {
            tracing::info!(plugin_id, rejected = report.total_rejected, "Python hook 投递恢复");
        }
        let inflight = self.config.max_inflight_hooks - self.inflight.available_permits();
        self.peak.fetch_max(inflight, Ordering::Relaxed);
        tracing::debug!(
            plugin_id,
            inflight,
            peak = self.peak.load(Ordering::Relaxed),
            rejected = self.rejected.load(Ordering::Relaxed),
            "接纳 Python hook"
        );
        Some(Admission {
            owner: self.clone(),
            quota,
            plugin_id: plugin_id.to_owned(),
            queued_at: Instant::now(),
            _global: global,
            _plugin: plugin,
        })
    }
    pub fn forget(&self, plugin_id: &str) { self.plugins.lock().unwrap().remove(plugin_id); }
}
impl Admission {
    pub async fn execute(self) -> Execution {
        // 先取单插件执行额度，避免慢插件占住全局额度等待自身空位。
        let plugin = self.quota.running.clone().acquire_owned().await.unwrap();
        let global = self.owner.running.clone().acquire_owned().await.unwrap();
        let running =
            self.owner.config.max_concurrent_hooks - self.owner.running.available_permits();
        let inflight =
            self.owner.config.max_inflight_hooks - self.owner.inflight.available_permits();
        tracing::debug!(plugin_id = %self.plugin_id, queue_ms = self.queued_at.elapsed().as_millis(), running, waiting = inflight.saturating_sub(running), "执行 Python hook");
        Execution {
            _admission: self,
            _global: global,
            _plugin: plugin,
            started: Instant::now(),
        }
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        tracing::debug!(plugin_id = %self._admission.plugin_id, execution_ms = self.started.elapsed().as_millis(), "Python hook 完成");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_config_absorbs_normal_bursts_and_per_plugin_overload_does_not_block_others() {
        let config: PyConfig =
            toml::from_str("plugin_path = 'unused'\nconfig_path = 'unused'\n").unwrap();
        config.validate().unwrap();
        let limits = HookLimits::new(config);
        let mut burst = Vec::new();
        for plugin in 0..8 {
            for _ in 0..64 {
                burst.push(limits.try_admit(&format!("plugin-{plugin}")).unwrap());
            }
        }
        assert_eq!(limits.rejected.load(Ordering::Relaxed), 0);
        drop(burst);
        let slow = (0..128).map(|_| limits.try_admit("slow").unwrap()).collect::<Vec<_>>();
        assert!(limits.try_admit("slow").is_none());
        assert!(limits.try_admit("healthy").is_some());
        drop(slow);
        assert!(limits.try_admit("slow").is_some());
    }
    #[tokio::test(start_paused = true)]
    async fn execution_limits_queue_without_rejection_and_cancelled_wait_returns_admission() {
        let limits = HookLimits::new(PyConfig {
            max_concurrent_hooks: 2,
            max_concurrent_hooks_per_plugin: 1,
            max_inflight_hooks: 4,
            max_inflight_hooks_per_plugin: 3,
            ..Default::default()
        });
        let first = limits.try_admit("slow").unwrap().execute().await;
        let waiting = limits.try_admit("slow").unwrap();
        let healthy = limits.try_admit("healthy").unwrap().execute().await;
        assert_eq!(limits.running.available_permits(), 0);
        assert!(tokio::time::timeout(Duration::from_millis(1), waiting.execute()).await.is_err());
        assert_eq!(limits.rejected.load(Ordering::Relaxed), 0);
        assert_eq!(limits.inflight.available_permits(), 2);
        drop((first, healthy));
        assert_eq!(limits.inflight.available_permits(), 4);
        let full = (0..4).map(|n| limits.try_admit(&n.to_string()).unwrap()).collect::<Vec<_>>();
        assert!(limits.try_admit("overflow").is_none());
        drop(full);
        assert!(limits.try_admit("overflow").is_some());
    }
}
