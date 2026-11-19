//! 长连接适配器的事件订阅和短期限纯文本发送；不把旧 client 暴露到插件线程。
use crate::{
    data_struct::ica::messages::SendMessage,
    py::{
        binding::RuntimeBinding,
        call::{PluginIdentity, current_plugin_identity},
        host::PluginHost,
        ica_connection::ConnectionState,
        plugin::LifecycleState,
    },
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};
use rust_socketio::asynchronous::Client;
use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, watch};

fn failed(code: &'static str) -> PyErr { PyRuntimeError::new_err(code) }
struct WatchState {
    first: bool,
    connection: watch::Receiver<ConnectionState<Client>>,
    lifecycle: watch::Receiver<()>,
}
#[pyclass(frozen, name = "IcaConnectionState")]
pub struct IcaConnectionStatePy {
    #[pyo3(get)]
    pub generation: u64,
    #[pyo3(get)]
    pub account: Option<i64>,
    #[pyo3(get)]
    pub ready: bool,
    #[pyo3(get)]
    pub plugin_active: bool,
}
#[pyclass(frozen, name = "IcaConnectionWatch")]
pub struct IcaConnectionWatchPy {
    host: Weak<PluginHost>,
    identity: PluginIdentity,
    state: Mutex<WatchState>,
    stopped: watch::Sender<bool>,
    slot: Mutex<Option<OwnedSemaphorePermit>>,
}
impl IcaConnectionWatchPy {
    pub fn new(host: &Arc<PluginHost>, identity: PluginIdentity) -> PyResult<Self> {
        let slot = host
            .ica_watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| failed("ICA_WATCH_CAPACITY"))?;
        Ok(Self {
            host: Arc::downgrade(host),
            identity,
            state: Mutex::new(WatchState {
                first: true,
                connection: host.ica_connection.subscribe(),
                lifecycle: host.lifecycle_changed.subscribe(),
            }),
            stopped: watch::channel(false).0,
            slot: Mutex::new(Some(slot)),
        })
    }
}
#[pymethods]
impl IcaConnectionWatchPy {
    /// 首次立即返回快照，之后只等变化；close/宿主停止返回 None。释放 GIL。
    pub fn wait_changed(&self, py: Python<'_>) -> PyResult<Option<IcaConnectionStatePy>> {
        py.detach(|| {
            let Some(host) = self.host.upgrade() else {
                return Ok(None);
            };
            let mut state =
                self.state.try_lock().map_err(|_| failed("ICA_WATCH_ALREADY_WAITING"))?;
            let mut stopped = self.stopped.subscribe();
            host.context.runtime.block_on(async {
                if !state.first && !*stopped.borrow() && host.is_accepting() {
                    let WatchState {
                        connection,
                        lifecycle,
                        ..
                    } = &mut *state;
                    tokio::select! {
                        _ = connection.changed() => {},
                        _ = lifecycle.changed() => {},
                        _ = stopped.changed() => {},
                    }
                }
                state.first = false;
                if *stopped.borrow() || !host.is_accepting() {
                    return Ok(None);
                }
                let connection = state.connection.borrow_and_update().clone();
                state.lifecycle.borrow_and_update();
                let storage = host.storage.lock().await;
                let owner = storage.storage.get(&self.identity.plugin_id);
                let active = owner.is_some_and(|p| {
                    p.state() == LifecycleState::Active
                        && p.generation() == self.identity.generation
                });
                let pending = owner.is_some_and(|p| {
                    p.state() == LifecycleState::Draining
                        && (p.generation() == self.identity.generation
                            || p.generation().checked_add(1) == Some(self.identity.generation))
                });
                // 候选尚未提交可以等待；已禁用、移除或被更高代替换则永久结束。
                if !active && !pending {
                    return Ok(None);
                }
                Ok(Some(IcaConnectionStatePy {
                    generation: connection.generation,
                    account: connection.account,
                    ready: active && connection.ready(),
                    plugin_active: active,
                }))
            })
        })
    }

    /// 返回 true 只表示 Socket.IO emit 受理，不是 QQ 确认；任何异常都可能已外发。
    pub fn send_text(
        &self,
        py: Python<'_>,
        generation: u64,
        account: i64,
        room_id: i64,
        text: String,
        timeout: f64,
    ) -> PyResult<bool> {
        if !timeout.is_finite()
            || timeout <= 0.0
            || timeout > 5.0
            || room_id == 0
            || account <= 0
            || account > 9_007_199_254_740_991
            || !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&room_id)
            || text.len() > 120 * 1024
        {
            return Err(PyValueError::new_err("ICA_INVALID_SEND_BUDGET_OR_TARGET"));
        }
        // 从进入原生方法开始计时；释放 GIL 和等宿主锁也不能重授预算。
        let deadline = std::time::Instant::now() + Duration::from_secs_f64(timeout);
        py.detach(|| {
            let host = self.host.upgrade().ok_or_else(|| failed("ICA_HOST_STOPPED"))?;
            let mut stopped = self.stopped.subscribe();
            host.context.runtime.block_on(async {
                tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
                    if *stopped.borrow() || !host.is_accepting() {
                        return Err(failed("ICA_WATCH_CLOSED"));
                    }
                    let storage = host.storage.lock().await;
                    if !storage.storage.get(&self.identity.plugin_id).is_some_and(|p| {
                        p.state() == LifecycleState::Active
                            && p.generation() == self.identity.generation
                    }) {
                        return Err(failed("ICA_PLUGIN_GENERATION_STALE"));
                    }
                    let admission = host
                        .limits
                        .try_admit(&self.identity.plugin_id)
                        .ok_or_else(|| failed("ICA_SEND_CAPACITY"))?;
                    // 和 Draining 的注册表锁建立屏障，实际发送属于被排空代的在途任务。
                    let _ticket = host.tasks.hook(self.identity.clone());
                    drop(storage);
                    let sending = async {
                        let _execution = admission.execute().await;
                        let storage = host.storage.lock().await;
                        if !host.is_accepting()
                            || !storage.storage.get(&self.identity.plugin_id).is_some_and(|p| {
                                p.state() == LifecycleState::Active
                                    && p.generation() == self.identity.generation
                            })
                        {
                            return Err(failed("ICA_PLUGIN_GENERATION_STALE"));
                        }
                        let current = host.ica_connection.current();
                        if !current.ready()
                            || current.generation != generation
                            || current.account != Some(account)
                        {
                            return Err(failed("ICA_CONNECTION_STALE"));
                        }
                        let client =
                            current.client.ok_or_else(|| failed("ICA_CONNECTION_STALE"))?;
                        drop(storage);
                        let message = SendMessage::new(text, room_id, None);
                        let body = serde_json::to_value(message)
                            .map_err(|_| failed("ICA_SEND_ENCODING"))?;
                        // timeout 的内层 Future 可能先被 poll；真正外发前仍须复查绝对期限。
                        if std::time::Instant::now() >= deadline || *self.stopped.borrow() {
                            return Err(failed("ICA_SEND_UNCERTAIN"));
                        }
                        client
                            .emit("sendMessage", body)
                            .await
                            .map_err(|_| failed("ICA_SEND_UNCERTAIN"))?;
                        Ok(true)
                    };
                    tokio::select! {
                        result = sending => result,
                        _ = stopped.changed() => Err(failed("ICA_WATCH_CLOSED")),
                    }
                })
                .await
                .map_err(|_| failed("ICA_SEND_UNCERTAIN"))?
            })
        })
    }
    pub fn close(&self) {
        self.stopped.send_replace(true);
        self.slot.lock().unwrap().take();
    }
}

#[pyfunction]
pub fn watch_ica_connection(py: Python<'_>) -> PyResult<IcaConnectionWatchPy> {
    let identity =
        current_plugin_identity().ok_or_else(|| failed("ICA_WATCH_REQUIRES_PLUGIN_CONTEXT"))?;
    RuntimeBinding::from_module(py)?
        .with_host(py, |host| IcaConnectionWatchPy::new(host, identity))?
}
