//! 与本体保持一致的申请处理、已读上报和 QQ NT 图片地址刷新接口。

use futures_util::FutureExt;
use rust_socketio::{Payload, asynchronous::Client};
use serde_json::{Value as JsonValue, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

use super::ack_payload_values;

fn request_arguments(
    request_type: &str,
    flag: &str,
    accept: bool,
) -> Result<Vec<JsonValue>, String> {
    if !matches!(request_type, "friend" | "group") || flag.trim().is_empty() {
        return Err("申请类型必须为 friend/group，且 flag 不能为空".into());
    }
    Ok(vec![json!(request_type), json!(flag), json!(accept)])
}

/// 处理申请。成功仅表示请求已发送，不代表 QQ 已批准或拒绝。
pub async fn handle_request(
    client: &Client,
    request_type: &str,
    flag: &str,
    accept: bool,
) -> Result<(), String> {
    let arguments = request_arguments(request_type, flag, accept)?;
    client
        .emit("handleRequest", arguments)
        .await
        .map_err(|error| format!("发送申请处理请求失败: {error}"))
}

/// 主动上报 QQ 已读，不自动清除 Bridge 会话未读数。
pub async fn report_read(client: &Client, message_id: &str) -> Result<(), String> {
    if message_id.trim().is_empty() {
        return Err("已读消息 ID 不能为空".into());
    }
    client
        .emit("reportRead", json!(message_id))
        .await
        .map_err(|error| format!("发送已读上报失败: {error}"))
}

/// 向当前客户端绑定的 Bridge 请求图片新地址；不下载资源，不自动重试。
pub async fn get_nt_pic_url_by_file_id(
    client: &Client,
    file_id: &str,
    app_id: &str,
) -> Result<String, String> {
    if file_id.trim().is_empty() {
        return Err("图片 file_id 不能为空".into());
    }
    const TIMEOUT: Duration = Duration::from_secs(12);
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    client
        .emit_with_ack(
            "getNTPicURLbyFileid",
            vec![json!(file_id), json!(app_id)],
            TIMEOUT,
            move |payload, _| {
                let sender = sender.clone();
                async move {
                    if let Ok(mut sender) = sender.lock()
                        && let Some(sender) = sender.take()
                    {
                        let _ = sender.send(parse_image_url_ack(payload));
                    }
                }
                .boxed()
            },
        )
        .await
        .map_err(|error| format!("发送图片地址刷新请求失败: {error}"))?;
    tokio::time::timeout(TIMEOUT, receiver)
        .await
        .map_err(|_| "图片地址刷新 ACK 等待超时".to_string())?
        .map_err(|_| "图片地址刷新 ACK 通道提前关闭".to_string())?
}

fn parse_image_url_ack(payload: Payload) -> Result<String, String> {
    let value = ack_payload_values(payload).into_iter().next().unwrap_or(JsonValue::Null);
    let url = value
        .as_str()
        .filter(|url| {
            reqwest::Url::parse(url).is_ok_and(|parsed| {
                matches!(parsed.scheme(), "https" | "http")
                    && parsed.host_str().is_some()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
            })
        })
        .ok_or_else(|| "Bridge 未返回有效的 HTTP(S) 图片地址".to_string())?;
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_arguments_keep_bridge_order_and_explicit_rejection() {
        assert_eq!(
            request_arguments("friend", "test-flag", false).unwrap(),
            vec![json!("friend"), json!("test-flag"), json!(false)]
        );
        assert!(request_arguments("group", "test-flag", true).is_ok());
        assert!(request_arguments("other", "test-flag", true).is_err());
        assert!(request_arguments("friend", "  ", true).is_err());
    }

    #[test]
    fn image_url_ack_unwraps_socket_arguments_and_rejects_failure_values() {
        let url = "https://example.invalid/image";
        for payload in [Payload::Text(vec![json!(url)]), Payload::Text(vec![json!([url])])] {
            assert_eq!(parse_image_url_ack(payload).unwrap(), url);
        }
        for value in [
            json!(""),
            json!("error"),
            json!("file:///test"),
            json!("https://"),
            json!("https://user:password@example.invalid/image"),
            json!({"error": "failed"}),
            JsonValue::Null,
        ] {
            assert!(parse_image_url_ack(Payload::Text(vec![value])).is_err());
        }
    }
}
