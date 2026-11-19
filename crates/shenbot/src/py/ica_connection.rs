//! 主连接的有界最新状态；连接代际隔离迟到事件，订阅不轮询、不持有聊天数据。
use tokio::sync::watch;

#[derive(Clone)]
pub struct ConnectionState<T: Clone> {
    pub generation: u64,
    pub open: bool,
    pub account: Option<i64>,
    pub online: bool,
    pub client: Option<T>,
}
impl<T: Clone> ConnectionState<T> {
    pub fn ready(&self) -> bool {
        self.open && self.online && self.account.is_some_and(|id| id > 0) && self.client.is_some()
    }
}
#[derive(Clone)]
pub struct Connection<T: Clone> {
    changed: watch::Sender<ConnectionState<T>>,
}
pub struct ConnectionGuard<T: Clone> {
    pub generation: u64,
    connection: Connection<T>,
}
impl<T: Clone> Default for Connection<T> {
    fn default() -> Self {
        Self {
            changed: watch::channel(ConnectionState {
                generation: 0,
                open: false,
                account: None,
                online: false,
                client: None,
            })
            .0,
        }
    }
}
impl<T: Clone> Connection<T> {
    pub fn subscribe(&self) -> watch::Receiver<ConnectionState<T>> { self.changed.subscribe() }
    pub fn current(&self) -> ConnectionState<T> { self.changed.borrow().clone() }
    pub fn begin(&self) -> ConnectionGuard<T> {
        let mut generation = 0;
        self.changed.send_modify(|state| {
            generation = state.generation.checked_add(1).expect("ICA 连接代际耗尽");
            *state = ConnectionState {
                generation,
                open: true,
                account: None,
                online: false,
                client: None,
            };
        });
        ConnectionGuard {
            generation,
            connection: self.clone(),
        }
    }
    pub fn online_data(&self, generation: u64, client: T, account: Option<i64>, online: bool) {
        self.changed.send_if_modified(|state| {
            if !state.open || state.generation != generation {
                return false;
            }
            let account = account.filter(|id| *id > 0);
            if state.client.is_some() && state.account == account && state.online == online {
                return false;
            }
            state.client = Some(client);
            state.account = account;
            state.online = online;
            true
        });
    }
    pub fn set_online(&self, generation: u64, online: bool) {
        self.changed.send_if_modified(|state| {
            if !state.open || state.generation != generation || state.online == online {
                return false;
            }
            state.online = online;
            true
        });
    }
    pub fn disconnect(&self, generation: u64) {
        self.changed.send_if_modified(|state| {
            if !state.open || state.generation != generation {
                return false;
            }
            state.open = false;
            state.online = false;
            state.account = None;
            state.client = None;
            true
        });
    }
}
impl<T: Clone> Drop for ConnectionGuard<T> {
    fn drop(&mut self) { self.connection.disconnect(self.generation); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn socket_connected_is_not_ready_without_authenticated_account_and_online_data() {
        let connection = Connection::default();
        let mut changes = connection.subscribe();
        let guard = connection.begin();
        changes.borrow_and_update();
        connection.set_online(guard.generation, true);
        assert!(!connection.current().ready());
        connection.online_data(guard.generation, (), Some(10001), true);
        assert!(connection.current().ready());
        changes.borrow_and_update();
        connection.online_data(guard.generation, (), Some(10001), true);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), changes.changed())
                .await
                .is_err()
        );
        connection.set_online(guard.generation, false);
        changes.changed().await.unwrap();
        assert!(!connection.current().ready());
    }
    #[test]
    fn late_events_and_old_guard_cannot_revive_or_disconnect_new_generation() {
        let connection = Connection::default();
        let old = connection.begin();
        connection.online_data(old.generation, "old", Some(10001), true);
        connection.disconnect(old.generation);
        connection.online_data(old.generation, "late", Some(10001), true);
        assert!(!connection.current().ready());
        let current = connection.begin();
        connection.online_data(current.generation, "current", Some(10002), true);
        connection.online_data(old.generation, "late", Some(10001), false);
        drop(old);
        assert_eq!(connection.current().client, Some("current"));
        assert!(connection.current().ready());
        drop(current);
        assert!(!connection.current().ready());
    }
}
