//! Scheduler 继承明确的插件归属，到期回调走同一容量保护。
use crate::py::{binding::RuntimeBinding, call::current_plugin_identity, plugin::LifecycleState};
use pyo3::{PyTraverseError, PyVisit, exceptions::PyRuntimeError, prelude::*, types::PyFunction};
use std::{sync::Arc, time::Duration};

#[pyclass]
#[pyo3(name = "Scheduler")]
pub struct SchedulerPy {
    pub callback: Py<PyFunction>,
    pub schdule_time: Duration,
    binding: RuntimeBinding,
}
#[pymethods]
impl SchedulerPy {
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.callback)
    }
    #[new]
    pub fn new(func: Bound<'_, PyFunction>, schdule_time: Duration) -> PyResult<Self> {
        let binding = RuntimeBinding::from_module(func.py())?;
        Ok(Self {
            callback: func.unbind(),
            schdule_time,
            binding,
        })
    }
    pub fn start(&self, py: Python<'_>) -> PyResult<()> {
        let identity = current_plugin_identity().ok_or_else(|| {
            PyRuntimeError::new_err("Scheduler.start() 必须从受管理的插件函数调用")
        })?;
        let callback = Arc::new(self.callback.clone_ref(py));
        let wait = self.schdule_time;
        self.binding.with_host(py, move |host| {
            let storage = host.storage.blocking_lock();
            let allowed = storage.storage.get(&identity.plugin_id).is_some_and(|p| {
                (p.state() == LifecycleState::Active && p.generation() == identity.generation)
                || (p.state() == LifecycleState::Draining &&
                    (p.generation() == identity.generation || p.generation().saturating_add(1) == identity.generation))
            });
            if !allowed || !host.is_accepting() { return Err(PyRuntimeError::new_err("插件正在排空、禁用或服务关闭")); }
            let ticket = host.tasks.scheduler(identity.clone());
            let id = ticket.scheduler_id();
            let worker = host.clone();
            let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
            let task = host.context.runtime.spawn(async move {
                let _ticket = ticket;
                if gate_rx.await.is_err() { return; }
                // on_load 的候选代和回退旧代都只在提交 Active 后开始计时。
                loop {
                    let state = worker.storage.lock().await.storage.get(&identity.plugin_id)
                        .map(|p| (p.state(), p.generation()));
                    match state {
                        Some((LifecycleState::Active, generation)) if generation == identity.generation => break,
                        Some((LifecycleState::Draining, generation)) if generation == identity.generation || generation.saturating_add(1) == identity.generation => tokio::time::sleep(Duration::from_millis(10)).await,
                        _ => return,
                    }
                }
                tokio::time::sleep(wait).await;
                let owner = identity.clone();
                worker.enqueue(identity, move || Python::attach(|py| {
                    if let Err(error) = callback.call0(py) { tracing::warn!(plugin_id = %owner.plugin_id, error = %error, "Scheduler 回调失败"); }
                })).await;
            });
            host.tasks.attach_scheduler(id, task.abort_handle());
            drop(storage);
            let _ = gate_tx.send(());
            Ok(())
        })?
    }
}
