//! Icalingua bridge 的鉴权、消息发送和群管理请求封装。

use crate::MainStatus;
use crate::data_struct::ica::messages::{DeleteMessage, SendMessage};
use crate::data_struct::ica::{RoomId, RoomIdTrait, UserId};
use crate::error::{ClientResult, IcaError};

use colored::Colorize;
use ed25519_dalek::{Signature, Signer, SigningKey};
use futures_util::future::BoxFuture;
use rust_socketio::Payload;
use rust_socketio::asynchronous::Client;
use serde_json::{Value as JsonValue, json};
use std::sync::Arc;
use std::time::Duration;
use tracing::{Level, event, span};

/// bridge 允许的最长群禁言时长，单位为秒。
pub const GROUP_BAN_MAX_DURATION: u64 = 30 * 24 * 60 * 60;

/// 展开 rust-socketio 在 ACK payload 外层包装的参数数组。
fn ack_payload_values(payload: Payload) -> Vec<JsonValue> {
    match payload {
        Payload::Text(values) => {
            if let Some(JsonValue::Array(args)) = values.first()
                && values.len() == 1
            {
                return args.clone();
            }
            values
        }
        Payload::Binary(bytes) => vec![json!(bytes.to_vec())],
        _ => Vec::new(),
    }
}

/// 根据 Socket.IO 地址推导 bridge HTTP API 的基础地址。
fn ica_http_api_url() -> String {
    let host = MainStatus::global_config().ica().host;
    if let Some(rest) = host.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if let Some(rest) = host.strip_prefix("wss://") {
        format!("https://{rest}")
    } else {
        host
    }
}

/// 判断待发送 JSON 消息是否包含 Base64 图片。
fn json_has_b64img(value: &JsonValue) -> bool {
    value.get("b64img").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())
}

/// 通过 `requestToken` ACK 申请一次性 HTTP 消息发送令牌。
async fn request_send_token(client: &Client) -> Result<String, String> {
    let timeout = Duration::from_secs(30);
    let token = Arc::new(tokio::sync::Mutex::new(None::<String>));
    let token_cb = token.clone();

    let result = client
        .emit_with_ack(
            "requestToken",
            Vec::<JsonValue>::new(),
            timeout,
            move |payload: Payload, _client: Client| -> BoxFuture<'static, ()> {
                let token = token_cb.clone();
                Box::pin(async move {
                    let token_str = ack_payload_values(payload)
                        .into_iter()
                        .next()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    *token.lock().await = Some(token_str);
                })
            },
        )
        .await;

    if let Err(e) = result {
        return Err(format!("requestToken 发送失败: {e}"));
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut attempts = 0;
    loop {
        if let Some(token) = token.lock().await.take() {
            if token.is_empty() {
                return Err("requestToken 返回空 token".to_string());
            }
            return Ok(token);
        }
        attempts += 1;
        if attempts > 100 {
            return Err("requestToken 超时".to_string());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 使用 bridge HTTP API 和一次性令牌发送消息 JSON。
async fn http_send_message(
    api_base_url: &str,
    token: &str,
    value: &JsonValue,
) -> Result<(), String> {
    let api_base_url = api_base_url.trim_end_matches('/');
    let url = format!("{api_base_url}/api/{token}/sendMessage");
    let client = reqwest::Client::new();

    let response = client
        .post(&url)
        .json(value)
        .send()
        .await
        .map_err(|e| format!("HTTP POST 失败: {e}"))?;

    match response.status() {
        reqwest::StatusCode::ACCEPTED => Ok(()),
        reqwest::StatusCode::FORBIDDEN => Err("token 验证失败 (403)".to_string()),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE => Err("图片过大，无法发送 (413)".to_string()),
        status => Err(format!("sendMessage HTTP 错误: {status}")),
    }
}

/// 申请一次性令牌，并通过 HTTP API 发送包含 Base64 图片的消息。
async fn send_message_via_http(client: &Client, value: &JsonValue) -> Result<(), String> {
    let token = request_send_token(client).await?;
    let api_base_url = ica_http_api_url();
    http_send_message(&api_base_url, &token, value).await
}

/// 发送结构化 Icalingua 消息，并根据图片类型选择 Socket.IO 或 HTTP 通道。
pub async fn send_message(client: &Client, message: &SendMessage) -> bool {
    let value = message.as_value();
    if message.has_b64img() {
        match send_message_via_http(client, &value).await {
            Ok(_) => {
                event!(Level::DEBUG, "send_message {}", format!("{message:#?}").cyan());
                true
            }
            Err(e) => {
                event!(Level::WARN, "send_message faild:{}", e.red());
                false
            }
        }
    } else {
        match client.emit("sendMessage", value).await {
            Ok(_) => {
                event!(Level::DEBUG, "send_message {}", format!("{message:#?}").cyan());
                true
            }
            Err(e) => {
                event!(Level::WARN, "send_message faild:{}", format!("{e:#?}").red());
                false
            }
        }
    }
}

/// 发送原始 JSON 消息，并根据图片类型选择 Socket.IO 或 HTTP 通道。
pub async fn send_string_message(client: &Client, message: &JsonValue) -> bool {
    if json_has_b64img(message) {
        match send_message_via_http(client, message).await {
            Ok(_) => {
                event!(Level::INFO, "send_message {}", format!("{message:#?}").bright_blue());
                true
            }
            Err(e) => {
                event!(Level::WARN, "send_message faild:{}", e.red());
                false
            }
        }
    } else {
        match client.emit("sendMessage", message.clone()).await {
            Ok(_) => {
                event!(Level::INFO, "send_message {}", format!("{message:#?}").bright_blue());
                true
            }
            Err(e) => {
                event!(Level::WARN, "send_message faild:{}", format!("{e:#?}").red());
                false
            }
        }
    }
}

/// 请求 bridge 删除或撤回指定消息。
pub async fn delete_message(client: &Client, message: &DeleteMessage) -> bool {
    match client
        .emit("deleteMessage", vec![json!(message.room_id), json!(message.message_id)])
        .await
    {
        Ok(_) => {
            event!(Level::DEBUG, "delete_message {}", format!("{message:#?}").yellow());
            true
        }
        Err(e) => {
            event!(Level::WARN, "delete_message faild:{}", format!("{e:#?}").red());
            false
        }
    }
}

/// 解析 `requireAuth` payload、检查协议版本并向 bridge 提交签名。
async fn inner_sign(payload: Payload, client: &Client) -> ClientResult<(), IcaError> {
    let span = span!(Level::INFO, "signing icalingua");
    let _guard = span.enter();

    // 获取数据
    let require_data = match payload {
        Payload::Text(json_value) => Ok(json_value),
        _ => Err(IcaError::LoginFailed("Got a invalid payload".to_string())),
    }?;

    let (auth_key, version) = (&require_data[0], &require_data[1]);

    event!(
        Level::INFO,
        "服务器发过来的待签名key: {:?}, 服务端版本号: {:?}",
        auth_key,
        version
    );
    // 判定和自己的兼容版本号是否 一致
    let server_protocol_version = version
        .get("protocolVersion")
        .unwrap_or(&JsonValue::Null)
        .as_str()
        .unwrap_or("unknow");
    if server_protocol_version != crate::ica::ICA_PROTOCOL_VERSION {
        event!(
            Level::WARN,
            "服务器版本与兼容版本不一致\n服务器协议版本:{:?}\n兼容版本:{}",
            version.get("protocolVersion"),
            crate::ica::ICA_PROTOCOL_VERSION
        );
    }

    let auth_key = match &require_data.first() {
        Some(JsonValue::String(auth_key)) => Ok(auth_key),
        _ => Err(IcaError::LoginFailed("Got a invalid auth_key".to_string())),
    }?;

    let salt = hex::decode(auth_key).expect("Got an invalid salt from the server");
    // 签名
    let private_key = MainStatus::global_config().ica().private_key.clone();

    let array_key: [u8; 32] = hex::decode(private_key)
        .expect("配置文件设置的私钥不是一个有效的私钥, 无法使用hex解析")
        .try_into()
        .expect("配置文件设置的私钥不是一个有效的私钥, 无法转换为[u8; 32]数组");
    let signing_key: SigningKey = SigningKey::from_bytes(&array_key);
    let signature: Signature = signing_key.sign(salt.as_slice());

    // 发送签名
    let sign = signature.to_bytes().to_vec();
    client.emit("auth", sign).await.expect("发送签名信息失败");
    Ok(())
}

/// 处理 `requireAuth` 事件；签名或鉴权参数无效时终止当前任务。
pub async fn sign_callback(payload: Payload, client: Client) {
    inner_sign(payload, &client).await.expect("Faild to sign");
}

/// 向指定群发送群签到；私聊房间会直接拒绝该操作。
pub async fn send_room_sign_in(client: &Client, room_id: RoomId) -> bool {
    if room_id.is_chat() {
        event!(Level::WARN, "不能向私聊发送签到信息");
        return false;
    }
    let data = json!(room_id.abs());
    match client.emit("sendGroupSign", data).await {
        Ok(_) => {
            event!(Level::INFO, "已向群 {} 发送签到信息", room_id);
            true
        }
        Err(e) => {
            event!(Level::ERROR, "向群 {} 发送签到信息失败: {}", room_id, e);
            false
        }
    }
}

/// 向指定房间中的用户发送戳一戳。
pub async fn send_poke(client: &Client, room_id: RoomId, target: UserId) -> bool {
    let data = vec![json!(room_id), json!(target)];
    match client.emit("sendGroupPoke", data).await {
        Ok(_) => {
            event!(Level::INFO, "已向 {} 的 {} 发送戳一戳", room_id, target);
            true
        }
        Err(e) => {
            event!(Level::ERROR, "向 {} 的 {} 发送戳一戳失败: {}", room_id, target, e);
            false
        }
    }
}

/// 禁言指定群成员
///
/// `duration` 单位为秒，设为 0 时解除禁言，最大为 30 天。
pub async fn set_group_ban(
    client: &Client,
    room_id: RoomId,
    target: UserId,
    duration: u64,
) -> bool {
    if room_id.is_chat() {
        event!(Level::WARN, "不能在私聊中禁言用户");
        return false;
    }
    if duration > GROUP_BAN_MAX_DURATION {
        event!(
            Level::WARN,
            "禁言时长不能超过 30 天（{} 秒），收到 {} 秒",
            GROUP_BAN_MAX_DURATION,
            duration
        );
        return false;
    }
    let data = vec![json!(room_id.abs()), json!(target), json!(duration)];
    match client.emit("setGroupBan", data).await {
        Ok(_) => {
            event!(Level::INFO, "已在群 {} 禁言 {}，时长 {} 秒", room_id, target, duration);
            true
        }
        Err(e) => {
            event!(Level::ERROR, "在群 {} 禁言 {} 失败: {}", room_id, target, e);
            false
        }
    }
}
