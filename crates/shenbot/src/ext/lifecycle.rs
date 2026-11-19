//! 每个实例串行执行控制操作；事件派发只持有短时路由锁。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ica_ext_sdk::{
    Hello, HostToPlugin, PROTOCOL_VERSION, PluginToHost, Shutdown, ShutdownReason, Subscribe,
};
use tokio::sync::{OwnedMutexGuard, watch};
use tokio::time::{Instant, sleep_until, timeout, timeout_at};

use crate::config::{ExtPluginConfig, ReloadMode};

use super::process::{PluginProcess, PluginStdin, PreparedPlugin, ProcessRuntime};
use super::supervisor::{self, Monitor};
use super::{ExtHostServices, ExtPluginError, ExtPluginState, ExtPluginSummary};

#[derive(Clone, Copy)]
pub(super) enum Command {
    Start,
    Enable,
    Disable,
    Reload,
    Shutdown,
}

pub(super) struct Instance {
    config: ExtPluginConfig,
    handshake_timeout: Duration,
    grace_ms: u64,
    runtime: Arc<ProcessRuntime>,
    services: Arc<dyn ExtHostServices>,
    closing: Arc<AtomicBool>,
    operation: Arc<tokio::sync::Mutex<()>>,
    state: Mutex<State>,
}

struct State {
    enabled: bool,
    phase: ExtPluginState,
    active: Option<Arc<Running>>,
    name: Option<String>,
    version: Option<String>,
    last_error: Option<String>,
    next_generation: u64,
    retry_epoch: u64,
    retry_candidate: Option<Candidate>,
    restart: RestartPolicy,
}

#[derive(Clone)]
struct Candidate {
    generation: u64,
    prepared: PreparedPlugin,
}

struct Running {
    candidate: Candidate,
    /// 每次启动都不同，即使用同一份副本回滚也不接收旧进程的退出通知。
    run: Arc<()>,
    subscribe: Subscribe,
    stdin: PluginStdin,
    finished: watch::Receiver<bool>,
    force: watch::Sender<bool>,
}

#[derive(Default)]
struct RestartPolicy {
    recent: VecDeque<Instant>,
    next_delay_secs: u64,
}

impl RestartPolicy {
    fn register_failure(&mut self, now: Instant) -> Option<Duration> {
        let window = Duration::from_secs(30);
        if self.recent.back().is_none_or(|last| now.duration_since(*last) >= window) {
            self.next_delay_secs = 1;
        }
        self.recent.retain(|failure| now.duration_since(*failure) < window);
        self.recent.push_back(now);
        if self.recent.len() >= 5 {
            return None;
        }
        let delay = Duration::from_secs(self.next_delay_secs);
        self.next_delay_secs = (self.next_delay_secs * 2).min(16);
        Some(delay)
    }
}

impl Instance {
    pub(super) fn new(
        config: ExtPluginConfig,
        handshake_timeout: Duration,
        grace_ms: u64,
        runtime: Arc<ProcessRuntime>,
        services: Arc<dyn ExtHostServices>,
        closing: Arc<AtomicBool>,
    ) -> Self {
        let state = Mutex::new(State {
            enabled: config.enabled,
            phase: ExtPluginState::Stopped,
            active: None,
            name: None,
            version: None,
            last_error: None,
            next_generation: 1,
            retry_epoch: 0,
            retry_candidate: None,
            restart: RestartPolicy::default(),
        });
        Self {
            config,
            handshake_timeout,
            grace_ms,
            runtime,
            services,
            closing,
            operation: Arc::default(),
            state,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("外部插件状态锁中毒")
    }

    fn is_closing(&self) -> bool { self.closing.load(Ordering::Acquire) }

    pub(super) fn summary(&self) -> ExtPluginSummary {
        let state = self.state();
        ExtPluginSummary {
            id: self.config.id.clone(),
            name: state.name.clone(),
            version: state.version.clone(),
            enabled: state.enabled,
            state: state.phase,
            last_error: state.last_error.clone(),
            generation: state.active.as_ref().map(|active| active.candidate.generation),
        }
    }

    pub(super) fn dispatch(&self, event: &HostToPlugin, system_message: bool) {
        let state = self.state();
        let Some(active) = state.active.as_ref() else {
            return;
        };
        if !self.is_closing()
            && subscribed(&active.subscribe, event, system_message)
            && let Err(error) = active.stdin.try_send(event)
        {
            tracing::debug!(plugin_id = %self.config.id, %error, "外部插件事件未能入队");
        }
    }

    pub(super) async fn command(self: &Arc<Self>, command: Command) -> Result<(), ExtPluginError> {
        // 管理命令来自可取消的事件任务。已受理的热切必须完整执行，不能停在旧代退场后。
        let instance = self.clone();
        tokio::spawn(async move {
            let _operation = instance.operation.lock().await;
            instance.command_locked(command).await
        })
        .await
        .map_err(|error| ExtPluginError::new(format!("外部插件管理任务结束异常：{error}")))?
    }

    async fn command_locked(self: &Arc<Self>, command: Command) -> Result<(), ExtPluginError> {
        if self.is_closing() && !matches!(command, Command::Shutdown) {
            return Err(ExtPluginError::new("外部插件宿主正在关闭"));
        }
        match command {
            Command::Start => {
                {
                    let state = self.state();
                    if !state.enabled || state.phase != ExtPluginState::Stopped {
                        return Ok(());
                    }
                }
                self.start(None).await
            }
            Command::Enable => {
                {
                    let mut state = self.state();
                    state.enabled = true;
                    if state.active.is_some() {
                        return Ok(());
                    }
                    state.reset_retry();
                }
                self.start(None).await
            }
            Command::Disable | Command::Shutdown => {
                let disable = matches!(command, Command::Disable);
                let old = {
                    let mut state = self.state();
                    if disable {
                        state.enabled = false;
                    }
                    state.reset_retry();
                    state.take_route()
                };
                if let Some(old) = old {
                    let reason = if disable {
                        ShutdownReason::Disable
                    } else {
                        ShutdownReason::HostExit
                    };
                    self.drain(&old, reason).await;
                }
                self.state().phase = ExtPluginState::Stopped;
                Ok(())
            }
            Command::Reload => {
                self.state().reset_retry();
                self.reload_locked().await
            }
        }
    }

    pub(super) fn reserve_self_reload(
        &self,
        run: &Arc<()>,
    ) -> Result<OwnedMutexGuard<()>, ExtPluginError> {
        if !self.config.allow_self_reload {
            return Err(ExtPluginError::new("配置未授权插件自行重载"));
        }
        if self.is_closing() {
            return Err(ExtPluginError::new("外部插件宿主正在关闭"));
        }
        let operation = self
            .operation
            .clone()
            .try_lock_owned()
            .map_err(|_| ExtPluginError::new("外部插件正在执行其他管理操作"))?;
        let state = self.state();
        if !state.enabled || !state.is_current(run) || self.is_closing() {
            return Err(ExtPluginError::new("仅当前活动代可请求重载"));
        }
        Ok(operation)
    }

    pub(super) fn self_reload(self: Arc<Self>, operation: OwnedMutexGuard<()>) {
        tokio::spawn(async move {
            let _operation = operation;
            if let Err(error) = self.reload_locked().await {
                tracing::warn!(plugin_id = %self.config.id, %error, "外部插件自行重载失败");
            }
        });
    }

    async fn prepare(&self) -> Result<Candidate, ExtPluginError> {
        let generation = {
            let mut state = self.state();
            let generation = state.next_generation;
            state.next_generation += 1;
            generation
        };
        let runtime = self.runtime.clone();
        let config = self.config.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            runtime.prepare(
                &config.id,
                generation,
                &config.binary,
                &config.args,
                config.working_dir.as_deref(),
            )
        })
        .await
        .map_err(|error| ExtPluginError::new(format!("准备外部插件副本的任务结束异常：{error}")))?
        .map_err(|error| {
            ExtPluginError::new(format!("无法准备外部插件第 {generation} 代副本：{error}"))
        })?;
        Ok(Candidate {
            generation,
            prepared,
        })
    }

    async fn handshake(
        &self,
        candidate: &Candidate,
    ) -> Result<(PluginProcess, Hello), ExtPluginError> {
        let mut process = candidate.prepared.spawn().map_err(|error| {
            ExtPluginError::new(format!("无法启动外部插件第 {} 代：{error}", candidate.generation))
        })?;
        let handshake = timeout(self.handshake_timeout, async {
            let hello = match process.stdout.recv().await {
                Ok(Some(PluginToHost::Hello(hello))) => hello,
                Ok(_) => return Err(ExtPluginError::new("插件首帧必须是 hello")),
                Err(error) => {
                    return Err(ExtPluginError::new(format!("读取插件 hello 失败：{error}")));
                }
            };
            if hello.protocol_version != PROTOCOL_VERSION {
                return Err(ExtPluginError::new(format!(
                    "不支持的外部插件协议版本：{}",
                    hello.protocol_version
                )));
            }
            if hello.id != self.config.id {
                return Err(ExtPluginError::new("插件 hello ID 与配置不匹配"));
            }
            let ack = self.services.hello_ack().await;
            process.stdin.send(&HostToPlugin::HelloAck(ack)).await.map_err(|error| {
                ExtPluginError::new(format!("发送插件 hello_ack 失败：{error}"))
            })?;
            Ok(hello)
        })
        .await
        .unwrap_or_else(|_| Err(ExtPluginError::new("等待插件握手超时")));
        match handshake {
            Ok(hello) => Ok((process, hello)),
            Err(error) => {
                // 必须先收回失败进程，串行回滚才可以启动旧副本。
                if let Err(kill_error) = process.child.kill().await {
                    tracing::warn!(plugin_id = %self.config.id, %kill_error, "终止握手失败的外部插件时出错");
                }
                Err(error)
            }
        }
    }

    fn activate(
        self: &Arc<Self>,
        candidate: Candidate,
        process: PluginProcess,
        hello: Hello,
        diagnostic: Option<String>,
    ) {
        let generation = candidate.generation;
        let (force, force_rx) = watch::channel(false);
        let (finished_tx, finished) = watch::channel(false);
        let run = Arc::new(());
        let running = Arc::new(Running {
            candidate,
            run: run.clone(),
            subscribe: hello.subscribe,
            stdin: process.stdin.clone(),
            force,
            finished,
        });
        {
            // 与 dispatch 使用同一把锁：切换后不再向旧代提交新事件。
            let mut state = self.state();
            state.name = Some(hello.name);
            state.version = Some(hello.version);
            state.active = Some(running);
            state.phase = ExtPluginState::Active;
            state.last_error = diagnostic;
        }
        tokio::spawn(supervisor::run(Monitor {
            id: self.config.id.clone(),
            generation,
            instance: Arc::downgrade(self),
            run,
            process,
            services: self.services.clone(),
            force: force_rx,
            finished: finished_tx,
            write_timeout: self.handshake_timeout,
        }));
        tracing::info!(plugin_id = %self.config.id, generation, "外部插件已就绪");
    }

    async fn start(
        self: &Arc<Self>,
        retry_candidate: Option<Candidate>,
    ) -> Result<(), ExtPluginError> {
        self.state().phase = ExtPluginState::Starting;
        // 成功运行过的副本保留；首次握手失败则下次重新读取源，允许重新部署后自动恢复。
        let keep_snapshot = retry_candidate.is_some();
        let candidate = match retry_candidate {
            Some(candidate) => candidate,
            None => match self.prepare().await {
                Ok(candidate) => candidate,
                Err(error) => {
                    self.record_failure(&error, None);
                    return Err(error);
                }
            },
        };
        match self.handshake(&candidate).await {
            Ok((process, hello)) => {
                self.activate(candidate, process, hello, None);
                Ok(())
            }
            Err(error) => {
                self.record_failure(&error, keep_snapshot.then_some(candidate));
                Err(error)
            }
        }
    }

    async fn reload_locked(self: &Arc<Self>) -> Result<(), ExtPluginError> {
        let old = {
            let state = self.state();
            if !state.enabled {
                return Err(ExtPluginError::new("外部插件未启用"));
            }
            state.active.clone()
        };
        let Some(old) = old else {
            return self.start(None).await;
        };
        let candidate = self.prepare().await.inspect_err(|error| {
            self.state().last_error = Some(error.to_string());
        })?;
        let serial = matches!(self.config.reload_mode, ReloadMode::Serial);
        if serial {
            self.state().take_route();
            self.drain(&old, ShutdownReason::Reload).await;
            self.state().phase = ExtPluginState::Starting;
        }
        match self.handshake(&candidate).await {
            Ok((process, hello)) => {
                self.activate(candidate, process, hello, None);
                if !serial {
                    self.drain(&old, ShutdownReason::Reload).await;
                }
                Ok(())
            }
            Err(error) if !serial => {
                self.state().last_error = Some(error.to_string());
                Err(error)
            }
            Err(error) => {
                let fallback = old.candidate.clone();
                match self.handshake(&fallback).await {
                    Ok((process, hello)) => {
                        let error = ExtPluginError::new(format!(
                            "重载失败，已回滚到第 {} 代：{error}",
                            fallback.generation
                        ));
                        tracing::warn!(plugin_id = %self.config.id, %error);
                        self.activate(fallback, process, hello, Some(error.to_string()));
                        Err(error)
                    }
                    Err(rollback_error) => {
                        let error = ExtPluginError::new(format!(
                            "重载失败：{error}；回滚失败：{rollback_error}"
                        ));
                        self.record_failure(&error, Some(fallback));
                        Err(error)
                    }
                }
            }
        }
    }

    async fn drain(&self, running: &Running, reason: ShutdownReason) {
        let deadline = Instant::now() + Duration::from_millis(self.grace_ms);
        let mut finished = running.finished.clone();
        if timeout_at(deadline, async {
            if !*finished.borrow() {
                let shutdown = HostToPlugin::Shutdown(Shutdown {
                    reason,
                    grace_ms: self.grace_ms,
                });
                if let Err(error) = running.stdin.send(&shutdown).await {
                    tracing::debug!(plugin_id = %self.config.id, %error, "外部插件停机信号未送达");
                }
            }
            let _ = finished.wait_for(|done| *done).await;
        })
        .await
        .is_err()
        {
            tracing::warn!(plugin_id = %self.config.id, generation = running.candidate.generation, "外部插件排空超时，强制终止");
            let _ = running.force.send(true);
            let _ = finished.wait_for(|done| *done).await;
        }
    }

    pub(super) async fn exited(self: &Arc<Self>, run: &Arc<()>, description: String) {
        let _operation = self.operation.lock().await;
        let candidate = {
            let mut state = self.state();
            if !state.is_current(run) {
                return;
            }
            state.active.take().map(|active| active.candidate.clone())
        };
        self.record_failure(&ExtPluginError::new(description), candidate);
    }

    fn record_failure(self: &Arc<Self>, error: &ExtPluginError, candidate: Option<Candidate>) {
        tracing::warn!(plugin_id = %self.config.id, %error, "外部插件运行失败");
        let mut state = self.state();
        state.phase = ExtPluginState::Failed;
        state.last_error = Some(error.to_string());
        if !self.config.auto_restart || !state.enabled || self.is_closing() {
            return;
        }
        let now = Instant::now();
        let Some(delay) = state.restart.register_failure(now) else {
            tracing::error!(plugin_id = %self.config.id, "外部插件在 30 秒内失败 5 次，已熔断");
            return;
        };
        state.phase = ExtPluginState::Backoff;
        state.retry_candidate = candidate;
        state.retry_epoch += 1;
        let epoch = state.retry_epoch;
        let instance = Arc::downgrade(self);
        tokio::spawn(async move {
            sleep_until(now + delay).await;
            let Some(instance) = instance.upgrade() else {
                return;
            };
            let _operation = instance.operation.lock().await;
            let candidate = {
                let mut state = instance.state();
                if state.retry_epoch != epoch || instance.is_closing() {
                    return;
                }
                state.retry_candidate.take()
            };
            let _ = instance.start(candidate).await;
        });
    }
}

impl State {
    fn reset_retry(&mut self) {
        self.retry_epoch += 1;
        self.retry_candidate = None;
        self.restart = RestartPolicy::default();
    }

    fn is_current(&self, run: &Arc<()>) -> bool {
        self.active.as_ref().is_some_and(|active| Arc::ptr_eq(&active.run, run))
    }

    fn take_route(&mut self) -> Option<Arc<Running>> {
        let active = self.active.take();
        if active.is_some() {
            self.phase = ExtPluginState::Draining;
        }
        active
    }
}

fn subscribed(subscribe: &Subscribe, event: &HostToPlugin, system_message: bool) -> bool {
    match event {
        HostToPlugin::NewMessage(_) if system_message => subscribe.system_messages,
        HostToPlugin::NewMessage(message) => {
            subscribe.all_messages
                || subscribe
                    .commands
                    .iter()
                    .any(|prefix| !prefix.is_empty() && message.content.starts_with(prefix))
        }
        HostToPlugin::ConnectionState(_) => subscribe.connection_events,
        HostToPlugin::RoomsUpdated { .. } => subscribe.rooms_events,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_failures_back_off_and_trip_but_old_failures_leave_the_window() {
        let start = Instant::now();
        let mut fast = RestartPolicy::default();
        for (time, next) in [(0, 1), (1, 2), (3, 4), (7, 8)] {
            assert_eq!(
                fast.register_failure(start + Duration::from_secs(time)),
                Some(Duration::from_secs(next))
            );
        }
        assert!(fast.register_failure(start + Duration::from_secs(15)).is_none());

        let mut slow = RestartPolicy::default();
        for (time, next) in [(0, 1), (10, 2), (20, 4), (30, 8), (40, 16), (50, 16)] {
            assert_eq!(
                slow.register_failure(start + Duration::from_secs(time)),
                Some(Duration::from_secs(next))
            );
        }
        assert_eq!(
            slow.register_failure(start + Duration::from_secs(85)),
            Some(Duration::from_secs(1))
        );
    }
}
