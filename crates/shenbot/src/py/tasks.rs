//! 通过 RAII 完成通知跟踪真实任务寿命，登记时不全量扫描任务。
use super::call::PluginIdentity;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, task::AbortHandle};

#[derive(Default)]
struct State {
    next_id: u64,
    hooks: HashMap<PluginIdentity, usize>,
    schedulers: HashMap<u64, (PluginIdentity, Option<AbortHandle>)>,
    total: usize,
}
#[derive(Default)]
pub struct TaskTracker {
    state: Mutex<State>,
    changed: Notify,
}

pub struct TaskTicket {
    tracker: Arc<TaskTracker>,
    identity: PluginIdentity,
    scheduler_id: Option<u64>,
}
impl TaskTracker {
    pub fn hook(self: &Arc<Self>, identity: PluginIdentity) -> TaskTicket {
        let mut state = self.state.lock().unwrap();
        *state.hooks.entry(identity.clone()).or_default() += 1;
        state.total += 1;
        TaskTicket {
            tracker: self.clone(),
            identity,
            scheduler_id: None,
        }
    }
    pub fn scheduler(self: &Arc<Self>, identity: PluginIdentity) -> TaskTicket {
        let mut state = self.state.lock().unwrap();
        state.next_id += 1;
        let id = state.next_id;
        state.schedulers.insert(id, (identity.clone(), None));
        state.total += 1;
        TaskTicket {
            tracker: self.clone(),
            identity,
            scheduler_id: Some(id),
        }
    }
    pub fn attach_scheduler(&self, id: u64, abort: AbortHandle) {
        if let Some((_, slot)) = self.state.lock().unwrap().schedulers.get_mut(&id) {
            *slot = Some(abort);
        }
    }
    pub fn abort_schedulers(&self, identity: Option<&PluginIdentity>) {
        let handles = self
            .state
            .lock()
            .unwrap()
            .schedulers
            .values()
            .filter(|(owner, _)| identity.is_none_or(|id| id == owner))
            .filter_map(|(_, handle)| handle.clone())
            .collect::<Vec<_>>();
        for handle in handles {
            handle.abort();
        }
    }
    pub fn total_len(&self) -> usize { self.state.lock().unwrap().total }
    pub async fn wait_hooks(&self, identity: &PluginIdentity, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if !self.state.lock().unwrap().hooks.contains_key(identity) {
                    break;
                }
                changed.await;
            }
        })
        .await
        .is_ok()
    }
    pub async fn wait_all(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.total_len() == 0 {
                return;
            }
            changed.await;
        }
    }
}
impl TaskTicket {
    pub fn scheduler_id(&self) -> u64 { self.scheduler_id.expect("不是 Scheduler") }
}
impl Drop for TaskTicket {
    fn drop(&mut self) {
        let mut state = self.tracker.state.lock().unwrap();
        if let Some(id) = self.scheduler_id {
            state.schedulers.remove(&id);
        } else if let Some(count) = state.hooks.get_mut(&self.identity) {
            *count -= 1;
            if *count == 0 {
                state.hooks.remove(&self.identity);
            }
        }
        state.total -= 1;
        drop(state);
        self.tracker.changed.notify_waiters();
    }
}

/// 生命周期事务独立于普通 hook 配额，也不会计入目标插件的排空计数。
#[derive(Default)]
pub struct Operations {
    state: Mutex<(bool, usize)>,
    changed: Notify,
}
pub struct OperationTicket {
    operations: Arc<Operations>,
}
impl Operations {
    pub fn begin(self: &Arc<Self>) -> Option<OperationTicket> {
        let mut state = self.state.lock().unwrap();
        if state.0 {
            return None;
        }
        state.1 += 1;
        Some(OperationTicket {
            operations: self.clone(),
        })
    }
    pub fn close(&self) { self.state.lock().unwrap().0 = true; }
    pub async fn wait_idle(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().unwrap().1 == 0 {
                return;
            }
            changed.await;
        }
    }
}
impl Drop for OperationTicket {
    fn drop(&mut self) {
        self.operations.state.lock().unwrap().1 -= 1;
        self.operations.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_operations_rejects_new_transactions_and_waits_existing_one() {
        let operations = Arc::new(Operations::default());
        let ticket = operations.begin().expect("关闭前应接纳事务");
        operations.close();
        assert!(operations.begin().is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(5), operations.wait_idle())
                .await
                .is_err()
        );
        drop(ticket);
        tokio::time::timeout(Duration::from_secs(1), operations.wait_idle())
            .await
            .unwrap();
    }
}
