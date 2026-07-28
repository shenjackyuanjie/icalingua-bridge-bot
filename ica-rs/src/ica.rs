//! Icalingua bridge 的 Socket.IO 客户端入口和事件注册。

pub mod client;
pub mod events;

use std::time::{Duration, Instant};

use colored::Colorize;
use futures_util::future::BoxFuture;
use rust_socketio::asynchronous::{Client, ClientBuilder};
use rust_socketio::{Event, Payload, TransportType, async_any_callback, async_callback};
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;
use tracing::{Level, event, span};

use crate::config::IcaConfig;
use crate::error::{ClientResult, IcaError};
use crate::{MainStatus, StopGetter, version_str};

pub const ICA_PROTOCOL_VERSION: &str = "2.26.0";
const MAX_RECONNECT_ATTEMPTS: usize = 5;
const MAX_RECONNECT_BACKOFF_SECS: u64 = 30;

#[derive(Debug)]
enum ConnectionSignal {
    Disconnected,
    AuthFailed(String),
    Fatal(String),
}

impl ConnectionSignal {
    fn terminal_error(self) -> Option<IcaError> {
        match self {
            Self::Disconnected => None,
            Self::AuthFailed(message) => Some(IcaError::AuthFailed(message)),
            Self::Fatal(message) => Some(IcaError::Fatal(message)),
        }
    }
}

/// 返回第 `attempt` 次重连前的指数退避时间。
pub(crate) fn reconnect_delay(attempt: usize) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5) as u32;
    Duration::from_secs((1_u64 << exponent).min(MAX_RECONNECT_BACKOFF_SECS))
}

#[derive(Debug, Default)]
struct ReconnectState {
    attempts: usize,
}

impl ReconnectState {
    fn connected(&mut self) { self.attempts = 0; }

    fn next_retry(&mut self) -> Option<(usize, Duration)> {
        if self.attempts >= MAX_RECONNECT_ATTEMPTS {
            return None;
        }
        self.attempts += 1;
        Some((self.attempts, reconnect_delay(self.attempts)))
    }
}

async fn wait_for_retry(delay: Duration, stop_receiver: &mut StopGetter) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = stop_receiver => false,
    }
}

fn payload_summary(payload: &Payload) -> String {
    match payload {
        Payload::Text(values) => values
            .first()
            .map(JsonValue::to_string)
            .unwrap_or_else(|| "无错误详情".to_string()),
        Payload::Binary(_) => "二进制错误详情".to_string(),
        _ => "无错误详情".to_string(),
    }
}

fn build_client(
    config: &IcaConfig,
    signal_tx: mpsc::UnboundedSender<ConnectionSignal>,
) -> ClientBuilder {
    let disconnect_tx = signal_tx.clone();
    let auth_failed_tx = signal_tx.clone();
    let fatal_tx = signal_tx.clone();
    let message_tx = signal_tx;

    ClientBuilder::new(config.host.clone())
        .transport_type(TransportType::Websocket)
        .on_any(async_any_callback!(events::any_event))
        .on(
            "disconnect",
            move |_payload: Payload, _client: Client| -> BoxFuture<'static, ()> {
                let signal_tx = disconnect_tx.clone();
                Box::pin(async move {
                    let _ = signal_tx.send(ConnectionSignal::Disconnected);
                })
            },
        )
        .on("requireAuth", async_callback!(client::sign_callback))
        .on("message", move |payload: Payload, client: Client| -> BoxFuture<'static, ()> {
            let signal_tx = message_tx.clone();
            Box::pin(async move {
                let is_auth_failed = matches!(
                    &payload,
                    Payload::Text(values)
                        if values.first().and_then(JsonValue::as_str) == Some("authFailed")
                );
                let summary = payload_summary(&payload);
                events::connect_callback(payload, client).await;
                if is_auth_failed {
                    let _ = signal_tx.send(ConnectionSignal::AuthFailed(summary));
                }
            })
        })
        .on("authSucceed", async_callback!(events::connect_callback))
        .on(
            "authFailed",
            move |payload: Payload, client: Client| -> BoxFuture<'static, ()> {
                let signal_tx = auth_failed_tx.clone();
                Box::pin(async move {
                    let summary = payload_summary(&payload);
                    events::connect_callback(payload, client).await;
                    let _ = signal_tx.send(ConnectionSignal::AuthFailed(summary));
                })
            },
        )
        .on("messageSuccess", async_callback!(events::success_message))
        .on("messageError", async_callback!(events::failed_message))
        .on("onlineData", async_callback!(events::get_online_data))
        .on("setOnline", async_callback!(events::set_online))
        .on("setOffline", async_callback!(events::set_offline))
        .on("setShutUp", async_callback!(events::set_shut_up))
        .on("setAllRooms", async_callback!(events::update_all_room))
        .on("updateRoom", async_callback!(events::update_room))
        .on("setMessages", async_callback!(events::set_messages))
        .on("addMessage", async_callback!(events::add_message))
        .on("deleteMessage", async_callback!(events::delete_message))
        .on("renewMessage", async_callback!(events::renew_message))
        .on("renewMessageURL", async_callback!(events::renew_message_url))
        .on("sendAddRequest", async_callback!(events::join_request))
        .on("notifyError", async_callback!(events::notify_error))
        .on("fatal", move |payload: Payload, client: Client| -> BoxFuture<'static, ()> {
            let signal_tx = fatal_tx.clone();
            Box::pin(async move {
                let summary = payload_summary(&payload);
                events::fatal_error(payload, client).await;
                let _ = signal_tx.send(ConnectionSignal::Fatal(summary));
            })
        })
        .on("requestSetup", async_callback!(events::request_setup))
        .on("login-verify", async_callback!(events::login_verify))
        .on("login-qrcodeLogin", async_callback!(events::login_qrcode))
        .on("login-smsCodeVerify", async_callback!(events::login_sms_code))
        .on("login-error", async_callback!(events::login_error))
        .on("login-slider", async_callback!(events::login_slider))
        .on("login", async_callback!(events::bridge_login))
}

/// 监督 ICA 连接，处理停止、断线重连和不可恢复的 bridge 错误。
pub async fn start_ica(
    config: &IcaConfig,
    mut stop_receiver: StopGetter,
) -> ClientResult<(), IcaError> {
    let span = span!(Level::INFO, "Icalingua Client");
    let _enter = span.enter();
    event!(Level::INFO, "ica-async-rs v{} initing", crate::ICA_VERSION);

    let mut reconnect = ReconnectState::default();
    let mut startup_notice_sent = false;

    'connect: loop {
        let (signal_tx, mut signal_rx) = mpsc::unbounded_channel();
        let started = Instant::now();
        let socket = match build_client(config, signal_tx).connect().await {
            Ok(client) => {
                event!(
                    Level::INFO,
                    "{}",
                    format!("socketio connected time: {:?}", started.elapsed()).on_cyan()
                );
                reconnect.connected();
                client
            }
            Err(error) => {
                event!(Level::ERROR, "socketio connect failed: {error}");
                let Some((attempt, delay)) = reconnect.next_retry() else {
                    return Err(IcaError::ReconnectExhausted {
                        attempts: MAX_RECONNECT_ATTEMPTS,
                        last_error: error.to_string(),
                    });
                };
                event!(
                    Level::WARN,
                    "{} 秒后进行第 {}/{} 次 ICA 重连",
                    delay.as_secs(),
                    attempt,
                    MAX_RECONNECT_ATTEMPTS
                );
                if wait_for_retry(delay, &mut stop_receiver).await {
                    continue 'connect;
                }
                return Ok(());
            }
        };

        if config.notice_start && !startup_notice_sent {
            startup_notice_sent = true;
            for room in &config.notice_room {
                let startup_msg = crate::data_struct::ica::messages::SendMessage::new(
                    format!("{}\n启动成功", version_str()),
                    *room,
                    None,
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
                event!(Level::INFO, "发送启动消息到房间: {room}");
                if let Err(error) =
                    socket.emit("sendMessage", serde_json::to_value(startup_msg).unwrap()).await
                {
                    event!(Level::INFO, "启动信息发送失败 房间:{room}|e:{error}");
                }
            }
        }

        event!(Level::INFO, "{}", "ica client waiting for stop signal".purple());
        let signal = tokio::select! {
            _ = &mut stop_receiver => {
                event!(Level::INFO, "{}", "socketio client stopping".yellow());
                let _ = socket.disconnect().await;
                return Ok(());
            }
            signal = signal_rx.recv() => signal,
        };

        MainStatus::global_ica_status_mut().clear_loaded_message_counts();
        let _ = socket.disconnect().await;
        if let Some(error) = signal.and_then(ConnectionSignal::terminal_error) {
            return Err(error);
        }
        let (_, delay) = reconnect.next_retry().expect("成功连接后第一次断线必须允许重连");
        event!(Level::WARN, "{} 秒后重连 ICA", delay.as_secs());
        if wait_for_retry(delay, &mut stop_receiver).await {
            continue 'connect;
        }
        return Ok(());
    }
}

#[cfg(test)]
mod reconnect_tests {
    use std::time::Duration;

    use super::{ConnectionSignal, ReconnectState, reconnect_delay, wait_for_retry};
    use crate::error::IcaError;

    #[test]
    fn reconnect_delay_is_exponential_and_capped() {
        let seconds: Vec<u64> = (1..=5).map(|attempt| reconnect_delay(attempt).as_secs()).collect();
        assert_eq!(seconds, [1, 2, 4, 8, 16]);
        assert_eq!(reconnect_delay(10).as_secs(), 30);
    }

    #[test]
    fn successful_connection_resets_attempts_and_fifth_retry_exhausts() {
        let mut state = ReconnectState::default();
        for expected in 1..=5 {
            assert_eq!(state.next_retry().unwrap().0, expected);
        }
        assert!(state.next_retry().is_none());
        state.connected();
        assert_eq!(state.next_retry().unwrap().0, 1);
    }

    #[tokio::test]
    async fn stop_interrupts_retry_wait() {
        let (stop_sender, mut stop_receiver) = tokio::sync::oneshot::channel();
        stop_sender.send(()).unwrap();
        assert!(!wait_for_retry(Duration::from_secs(60), &mut stop_receiver).await);
    }

    #[test]
    fn auth_and_fatal_are_terminal_but_disconnect_is_not() {
        assert!(ConnectionSignal::Disconnected.terminal_error().is_none());
        assert!(matches!(
            ConnectionSignal::AuthFailed("bad key".into()).terminal_error(),
            Some(IcaError::AuthFailed(_))
        ));
        assert!(matches!(
            ConnectionSignal::Fatal("bridge stopped".into()).terminal_error(),
            Some(IcaError::Fatal(_))
        ));
    }
}
