//! 单次 Bot 运行实例拥有的配置、状态及网络资源。
use crate::{config::BotConfig, status};
use std::sync::{Arc, RwLock};

pub struct BotContext {
    pub config: BotConfig,
    pub runtime: tokio::runtime::Handle,
    pub http: reqwest::Client,
    ica: RwLock<status::ica::MainStatus>,
    tailchat: RwLock<status::tailchat::MainStatus>,
}

impl BotContext {
    pub fn new(config: BotConfig, runtime: tokio::runtime::Handle) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(!config.enable_ica || config.ica.is_some(), "启用 ICA 时必须配置 ica");
        anyhow::ensure!(
            !config.enable_tailchat || config.tailchat.is_some(),
            "启用 Tailchat 时必须配置 tailchat"
        );
        anyhow::ensure!(!config.enable_py || config.py.is_some(), "启用 Python 时必须配置 py");
        if let Some(py) = &config.py {
            py.validate()?;
        }
        let ica = status::ica::MainStatus {
            enable: config.enable_ica,
            ..Default::default()
        };
        let tailchat = status::tailchat::MainStatus {
            enable: config.enable_tailchat,
            ..Default::default()
        };
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self::from_parts(config, runtime, http, ica, tailchat))
    }

    fn from_parts(
        config: BotConfig,
        runtime: tokio::runtime::Handle,
        http: reqwest::Client,
        ica: status::ica::MainStatus,
        tailchat: status::tailchat::MainStatus,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            runtime,
            http,
            ica: RwLock::new(ica),
            tailchat: RwLock::new(tailchat),
        })
    }

    #[cfg(test)]
    pub fn new_with_http_client(
        config: BotConfig,
        runtime: tokio::runtime::Handle,
        http: reqwest::Client,
    ) -> anyhow::Result<Arc<Self>> {
        let context = Self::new(config, runtime)?;
        Ok(Self::from_parts(
            context.config.clone(),
            context.runtime.clone(),
            http,
            context.read_ica(Clone::clone),
            context.read_tailchat(Clone::clone),
        ))
    }
    pub fn read_ica<T>(&self, read: impl FnOnce(&status::ica::MainStatus) -> T) -> T {
        read(&self.ica.read().expect("ICA 状态锁被污染"))
    }
    pub fn update_ica<T>(&self, update: impl FnOnce(&mut status::ica::MainStatus) -> T) -> T {
        update(&mut self.ica.write().expect("ICA 状态锁被污染"))
    }
    pub fn read_tailchat<T>(&self, read: impl FnOnce(&status::tailchat::MainStatus) -> T) -> T {
        read(&self.tailchat.read().expect("Tailchat 状态锁被污染"))
    }
    pub fn update_tailchat(&self, status: status::tailchat::MainStatus) {
        *self.tailchat.write().expect("Tailchat 状态锁被污染") = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_history_updates_and_connection_reset_are_isolated_between_runs() {
        let first =
            BotContext::new(BotConfig::default(), tokio::runtime::Handle::current()).unwrap();
        let second =
            BotContext::new(BotConfig::default(), tokio::runtime::Handle::current()).unwrap();
        second.update_ica(|state| {
            state.loaded_messages_count_by_room.insert(1, 7);
        });
        let mut workers = Vec::new();
        for _ in 0..16 {
            let context = first.clone();
            workers.push(tokio::spawn(async move {
                for _ in 0..100 {
                    context.update_ica(|state| {
                        *state.loaded_messages_count_by_room.entry(1).or_default() += 1
                    });
                    tokio::task::yield_now().await;
                }
            }));
        }
        for worker in workers {
            worker.await.unwrap();
        }
        assert_eq!(first.read_ica(|s| s.loaded_messages_count()), 1600);
        assert_eq!(second.read_ica(|s| s.loaded_messages_count()), 7);
        first.update_ica(|s| s.clear_loaded_message_counts());
        assert_eq!(second.read_ica(|s| s.loaded_messages_count()), 7);
    }
    #[tokio::test]
    async fn invalid_capacity_combinations_fail_before_starting_backends() {
        let base = crate::config::PyConfig::default();
        for py in [
            crate::config::PyConfig {
                max_inflight_hooks: 0,
                ..base.clone()
            },
            crate::config::PyConfig {
                max_concurrent_hooks: 2048,
                ..base.clone()
            },
            crate::config::PyConfig {
                max_concurrent_hooks_per_plugin: 64,
                ..base.clone()
            },
            crate::config::PyConfig {
                max_inflight_hooks_per_plugin: 2048,
                ..base.clone()
            },
            crate::config::PyConfig {
                reload_interval_ms: 0,
                ..base
            },
        ] {
            let config = BotConfig {
                py: Some(py),
                ..Default::default()
            };
            assert!(BotContext::new(config, tokio::runtime::Handle::current()).is_err());
        }
    }
}
