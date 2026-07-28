//! 暴露给 Python 插件的定时任务控制类型。

use std::time::Duration;

use pyo3::{
    Bound, Py, PyResult, PyTraverseError, PyVisit, Python, exceptions::PyRuntimeError, pyclass,
    pymethods, types::PyFunction,
};
use tracing::{Level, event};

use crate::py::{
    PY_PLUGIN_STORAGE,
    call::{PY_TASKS, TaskKind, TaskType, current_plugin_identity, with_plugin_context},
    plugin::LifecycleState,
};

#[derive(Debug)]
#[pyclass]
#[pyo3(name = "Scheduler")]
/// 用于计划任务的类
///
/// 给 Python 侧使用
///
/// add: 0.9.0
pub struct SchedulerPy {
    /// 回调函数
    ///
    /// 你最好不要把他清理掉
    pub callback: Py<PyFunction>,
    /// 预计等待时间
    pub schdule_time: Duration,
}

#[pymethods]
impl SchedulerPy {
    /// 遍历 Python 对象持有的引用。
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.callback)?;
        Ok(())
    }

    #[new]
    /// 创建并初始化对应的数据结构。
    pub fn new(func: Bound<'_, PyFunction>, schdule_time: Duration) -> Self {
        Self {
            callback: func.unbind(),
            schdule_time,
        }
    }

    /// 开始
    pub fn start(&self, py: Python<'_>) -> PyResult<()> {
        let identity = current_plugin_identity().ok_or_else(|| {
            PyRuntimeError::new_err(
                "Scheduler.start() 必须从受管理的插件 hook 或生命周期函数中调用",
            )
        })?;
        {
            let storage = PY_PLUGIN_STORAGE.blocking_lock();
            let active = storage.storage.get(&identity.plugin_id).is_some_and(|plugin| {
                (plugin.state() == LifecycleState::Active
                    && plugin.generation() == identity.generation)
                    || (plugin.state() == LifecycleState::Draining
                        && plugin.generation().saturating_add(1) == identity.generation)
            });
            if !active {
                return Err(PyRuntimeError::new_err("插件正在排空或已经禁用，不能创建 Scheduler"));
            }
        }

        let wait = self.schdule_time;
        let cb = self.callback.clone_ref(py);
        let task_identity = identity.clone();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            if gate_rx.await.is_err() {
                return;
            }
            // 候选代可在 `on_load` 中登记 Scheduler，但只有事务提交为 Active 后才开始计时。
            loop {
                let state = {
                    let storage = PY_PLUGIN_STORAGE.lock().await;
                    storage
                        .storage
                        .get(&task_identity.plugin_id)
                        .map(|plugin| (plugin.state(), plugin.generation()))
                };
                match state {
                    Some((LifecycleState::Active, generation))
                        if generation == task_identity.generation =>
                    {
                        break;
                    }
                    Some((LifecycleState::Draining, generation))
                        if generation.saturating_add(1) == task_identity.generation =>
                    {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    _ => return,
                }
            }
            tokio::time::sleep(wait).await;
            let callback_identity = task_identity.clone();
            let (callback_gate_tx, callback_gate_rx) = tokio::sync::oneshot::channel();
            let callback_handle = tokio::spawn(async move {
                if callback_gate_rx.await.is_err() {
                    return;
                }
                let _ = tokio::task::spawn_blocking(move || {
                    with_plugin_context(callback_identity, || {
                        Python::attach(|py| {
                            event!(Level::INFO, "正在调用计划 {:?}", wait);
                            if let Err(error) = cb.call0(py) {
                                event!(Level::WARN, "调用时出现错误 {error}");
                            }
                        });
                    });
                })
                .await;
            });

            let storage = PY_PLUGIN_STORAGE.lock().await;
            let active = storage.storage.get(&task_identity.plugin_id).is_some_and(|plugin| {
                plugin.state() == LifecycleState::Active
                    && plugin.generation() == task_identity.generation
            });
            if active {
                PY_TASKS.lock().await.register(
                    task_identity,
                    TaskKind::Hook(TaskType::SchedulerCallback),
                    callback_handle,
                );
                drop(storage);
                let _ = callback_gate_tx.send(());
            } else {
                callback_handle.abort();
            }
        });

        let storage = PY_PLUGIN_STORAGE.blocking_lock();
        let still_active = storage.storage.get(&identity.plugin_id).is_some_and(|plugin| {
            (plugin.state() == LifecycleState::Active && plugin.generation() == identity.generation)
                || (plugin.state() == LifecycleState::Draining
                    && plugin.generation().saturating_add(1) == identity.generation)
        });
        if !still_active {
            handle.abort();
            return Err(PyRuntimeError::new_err("插件正在排空或已经禁用，不能创建 Scheduler"));
        }
        PY_TASKS.blocking_lock().register(identity, TaskKind::Scheduler, handle);
        drop(storage);
        let _ = gate_tx.send(());
        Ok(())
    }
}
