//! 请求局部的一次性 ACK，截止时间覆盖登记与响应等待。
use futures_util::FutureExt;
use rust_socketio::{Payload, asynchronous::Client};
use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum AckError {
    Send(rust_socketio::Error),
    Timeout,
    Closed,
}
impl fmt::Display for AckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(error) => write!(f, "ACK 请求发送失败: {error}"),
            Self::Timeout => f.write_str("ACK 等待超时"),
            Self::Closed => f.write_str("ACK 通道提前关闭"),
        }
    }
}
impl std::error::Error for AckError {}
#[derive(Clone)]
struct Responder(Arc<Mutex<Option<oneshot::Sender<Payload>>>>);
impl Responder {
    fn respond(&self, payload: Payload) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|mut sender| sender.take())
            .is_some_and(|sender| sender.send(payload).is_ok())
    }
}
async fn wait_for_ack<F>(
    timeout: Duration,
    register: impl FnOnce(Responder) -> F,
) -> Result<Payload, AckError>
where
    F: Future<Output = Result<(), rust_socketio::Error>>,
{
    let (sender, receiver) = oneshot::channel();
    let responder = Responder(Arc::new(Mutex::new(Some(sender))));
    tokio::time::timeout(timeout, async {
        register(responder).await.map_err(AckError::Send)?;
        receiver.await.map_err(|_| AckError::Closed)
    })
    .await
    .map_err(|_| AckError::Timeout)?
}
pub async fn request(
    client: &Client,
    event: &str,
    data: Payload,
    timeout: Duration,
) -> Result<Payload, AckError> {
    wait_for_ack(timeout, |responder| async move {
        client
            .emit_with_ack(event, data, timeout, move |payload, _| {
                let responder = responder.clone();
                async move {
                    responder.respond(payload);
                }
                .boxed()
            })
            .await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test(start_paused = true)]
    async fn immediate_ack_has_no_polling_delay_and_duplicate_cannot_replace_it() {
        let start = tokio::time::Instant::now();
        let result = wait_for_ack(Duration::from_secs(3), |reply| async move {
            assert!(reply.respond(json!("first").into()));
            assert!(!reply.respond(json!("duplicate").into()));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(tokio::time::Instant::now(), start);
        assert!(matches!(result, Payload::Text(values) if values == vec![json!("first")]));
    }
    #[tokio::test(start_paused = true)]
    async fn deadline_includes_registration_and_late_ack_cannot_finish_another_request() {
        let (sender, receiver) = oneshot::channel();
        let start = tokio::time::Instant::now();
        let result = wait_for_ack(Duration::from_secs(3), |reply| async move {
            sender.send(reply).ok();
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        })
        .await;
        assert!(matches!(result, Err(AckError::Timeout)));
        assert_eq!(start.elapsed(), Duration::from_secs(3));
        assert!(!receiver.await.unwrap().respond(json!("late").into()));
        assert!(matches!(
            wait_for_ack(Duration::from_secs(3), |_| async { Ok(()) }).await,
            Err(AckError::Closed)
        ));
    }
}
