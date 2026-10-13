//! Icalingua bridge 的鉴权、消息发送和群管理请求封装。

use crate::context::BotContext;
use crate::data_struct::ica::group_members::GroupMember;
use crate::data_struct::ica::messages::{DeleteMessage, SendMessage};
use crate::data_struct::ica::{RoomId, RoomIdTrait, UserId};
use crate::error::{ClientResult, IcaError};

use colored::Colorize;
use ed25519_dalek::{Signature, Signer, SigningKey};

use rust_socketio::Payload;
use rust_socketio::asynchronous::Client;
use serde_json::{Value as JsonValue, json};

use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tracing::{Level, event, span};

mod ack;
pub mod bridge;
pub mod files;
mod protocol;
pub use protocol::{get_nt_pic_url_by_file_id, handle_request, report_read};

/// bridge 允许的最长群禁言时长，单位为秒。
pub const GROUP_BAN_MAX_DURATION: u64 = 30 * 24 * 60 * 60;
const GROUP_MEMBERS_ACK_TIMEOUT: Duration = Duration::from_secs(15);

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
fn ica_http_api_url(context: &BotContext) -> String {
    let host = context.config.ica().host.clone();
    if let Some(rest) = host.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if let Some(rest) = host.strip_prefix("wss://") {
        format!("https://{rest}")
    } else {
        host
    }
}

/// 判断待发送 JSON 消息是否包含需要通过 HTTP 上传的 Base64 媒体。
fn json_has_base64_media(value: &JsonValue) -> bool {
    value.get("media").and_then(JsonValue::as_array).is_some_and(|media| {
        media.iter().any(|attachment| {
            attachment
                .get("b64")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| !value.is_empty())
        })
    })
}

/// 通过 `requestToken` ACK 申请一次性 HTTP 消息发送令牌。
///
/// bridge 偶发会丢失/延迟 requestToken 的 ACK（如瞬时繁忙），
/// 这里分多次短等待重试：3 × 3.4s + 两次 200ms 间隔 ≈ 10.6s，与原先单次等待总时长相当，但显著提高成功率。
async fn request_send_token(client: &Client) -> Result<String, String> {
    retry_send_token(|| request_send_token_once(client)).await
}

async fn retry_send_token<F, Fut>(mut request: F) -> Result<String, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    const MAX_ATTEMPTS: usize = 3;
    let mut last_error = "requestToken 超时".to_string();
    for attempt in 1..=MAX_ATTEMPTS {
        match request().await {
            Ok(token) => return Ok(token),
            Err(error) => {
                if attempt < MAX_ATTEMPTS {
                    event!(
                        Level::WARN,
                        "requestToken 获取失败（第 {attempt}/{MAX_ATTEMPTS} 次）: {error}"
                    );
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                last_error = error;
            }
        }
    }
    Err(last_error)
}

async fn request_send_token_once(client: &Client) -> Result<String, String> {
    let payload = ack::request(
        client,
        "requestToken",
        Vec::<JsonValue>::new().into(),
        Duration::from_millis(3400),
    )
    .await
    .map_err(|error| format!("requestToken: {error}"))?;
    ack_payload_values(payload)
        .into_iter()
        .next()
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "requestToken 返回空 token".to_string())
}

/// 使用 bridge HTTP API 和一次性令牌发送消息 JSON。
async fn http_send_message(
    api_base_url: &str,
    token: &str,
    value: &JsonValue,
    context: &BotContext,
) -> Result<(), String> {
    let api_base_url = api_base_url.trim_end_matches('/');
    let url = format!("{api_base_url}/api/{token}/sendMessage");
    let client = &context.http;

    let response = client
        .post(&url)
        .json(value)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("HTTP POST 失败: {}", e.without_url()))?;

    match response.status() {
        reqwest::StatusCode::ACCEPTED => Ok(()),
        reqwest::StatusCode::FORBIDDEN => Err("token 验证失败 (403)".to_string()),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE => Err("媒体过大，无法发送 (413)".to_string()),
        status => Err(format!("sendMessage HTTP 错误: {status}")),
    }
}

/// 申请一次性令牌，并通过 HTTP API 发送包含 Base64 图片的消息。
async fn send_message_via_http(
    client: &Client,
    value: &JsonValue,
    context: &BotContext,
) -> Result<(), String> {
    let token = request_send_token(client).await?;
    let api_base_url = ica_http_api_url(context);
    http_send_message(&api_base_url, &token, value, context).await
}

/// "安全" 的 发送一条消息
///
/// 发送结构化 Icalingua 消息，并根据图片类型选择 Socket.IO 或 HTTP 通道。
pub async fn send_message(client: &Client, message: &SendMessage, context: &BotContext) -> bool {
    let value = message.as_value();
    if message.has_base64_media() {
        match send_message_via_http(client, &value, context).await {
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

/// "安全" 的 发一个 json 消息
///
/// 发送原始 JSON 消息，并根据图片类型选择 Socket.IO 或 HTTP 通道。
pub async fn send_string_message(
    client: &Client,
    message: &JsonValue,
    context: &BotContext,
) -> bool {
    if json_has_base64_media(message) {
        match send_message_via_http(client, message, context).await {
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

/// "安全" 的 删除一条消息
///
/// 草你妈草你妈 socketio 这他妈的接口设计的也太恶心了 ( 后半句话是 CodeGeex 补全的 )
///
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
async fn inner_sign(
    payload: Payload,
    client: &Client,
    context: &BotContext,
) -> ClientResult<(), IcaError> {
    let span = span!(Level::INFO, "signing icalingua");
    let _guard = span.enter();

    // 获取数据
    let require_data = match payload {
        Payload::Text(json_value) => Ok(json_value),
        _ => Err(IcaError::LoginFailed("Got a invalid payload".to_string())),
    }?;

    let (salt, version) = parse_auth_challenge(&require_data)?;
    let server_version = version.get("version").and_then(JsonValue::as_str).unwrap_or("unknown");
    event!(Level::INFO, "Bridge 服务版本: {server_version}");
    // 判定和自己的兼容版本号是否 一致
    let server_protocol_version = version
        .get("protocolVersion")
        .unwrap_or(&JsonValue::Null)
        .as_str()
        .unwrap_or("unknown");
    if server_protocol_version != crate::ica::ICA_PROTOCOL_VERSION {
        event!(
            Level::WARN,
            "服务器版本与兼容版本不一致\n服务器协议版本:{:?}\n兼容版本:{}",
            version.get("protocolVersion"),
            crate::ica::ICA_PROTOCOL_VERSION
        );
    }

    // 签名
    let private_key = context.config.ica().private_key.clone();

    let array_key: [u8; 32] = hex::decode(private_key)
        .map_err(|_| IcaError::LoginFailed("私钥不是有效的十六进制".into()))?
        .try_into()
        .map_err(|_| IcaError::LoginFailed("私钥必须为 32 字节".into()))?;
    let signing_key: SigningKey = SigningKey::from_bytes(&array_key);
    let signature: Signature = signing_key.sign(salt.as_slice());

    // 发送签名
    let sign = signature.to_bytes().to_vec();
    client.emit("auth", sign).await?;
    Ok(())
}

/// 版本信息可以缺失，但签名 challenge 必须是非空的十六进制字符串。
fn parse_auth_challenge(values: &[JsonValue]) -> Result<(Vec<u8>, JsonValue), IcaError> {
    let challenge = values
        .first()
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| IcaError::LoginFailed("Bridge 签名 challenge 缺失或类型错误".into()))?;
    let salt = hex::decode(challenge)
        .map_err(|_| IcaError::LoginFailed("Bridge 签名 challenge 不是有效的十六进制".into()))?;
    Ok((salt, values.get(1).cloned().unwrap_or(JsonValue::Null)))
}

/// 签名回调
/// 处理 `requireAuth` 事件；失败交给连接监督器，避免 panic 后连接永久等待。
pub async fn sign_callback(
    payload: Payload,
    client: Client,
    context: &BotContext,
) -> ClientResult<(), IcaError> {
    inner_sign(payload, &client, context).await
}

/// 向指定群发送签到信息
///
/// 只能是群啊, 不能是私聊
///
/// 私聊房间会直接拒绝该操作。
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

/// 向某个群/私聊的某个人发送戳一戳
///
/// 向指定房间中的用户发送戳一戳。
pub async fn send_poke(client: &Client, room_id: RoomId, target: UserId) -> bool {
    let Some((conversation_id, target)) = poke_target(room_id, target) else {
        event!(Level::WARN, "戳一戳目标无效，未发送请求");
        return false;
    };
    let data = vec![json!(conversation_id), json!(target)];
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

/// oicq 使用正群号，私聊的两个参数都必须指向会话对方。
fn poke_target(room_id: RoomId, target: UserId) -> Option<(i64, UserId)> {
    let conversation_id = room_id.checked_abs().filter(|id| *id > 0)?;
    if target <= 0 {
        return None;
    }
    Some((conversation_id, if room_id > 0 { room_id } else { target }))
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

/// 查询群聊的完整成员列表。
pub async fn get_group_members(
    client: &Client,
    room_id: RoomId,
) -> Result<Vec<GroupMember>, IcaError> {
    if !room_id.is_negative() {
        return Err(IcaError::InvalidGroupRoomId(room_id));
    }
    let group_id = room_id.checked_abs().ok_or(IcaError::InvalidGroupRoomId(room_id))?;
    let payload = ack::request(
        client,
        "getGroupMembers",
        vec![json!(group_id)].into(),
        GROUP_MEMBERS_ACK_TIMEOUT,
    )
    .await
    .map_err(|error| match error {
        ack::AckError::Send(error) => IcaError::SocketIoError(error),
        ack::AckError::Timeout => IcaError::GroupMembersTimeout(room_id),
        ack::AckError::Closed => IcaError::InvalidGroupMembersResponse("ACK 通道提前关闭".into()),
    })?;
    parse_group_members_ack(payload)
}

/// 查询当前仍处于禁言中的群成员。
pub async fn get_muted_group_members(
    client: &Client,
    room_id: RoomId,
) -> Result<Vec<GroupMember>, IcaError> {
    let members = get_group_members(client, room_id).await?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0);
    Ok(members.into_iter().filter(|member| member.is_muted_at(timestamp)).collect())
}

fn parse_group_members_ack(payload: Payload) -> Result<Vec<GroupMember>, IcaError> {
    let Payload::Text(mut values) = payload else {
        return Err(IcaError::InvalidGroupMembersResponse(
            "ACK payload 不是 JSON 文本".to_string(),
        ));
    };

    let members = if values.len() == 1 {
        match values.remove(0) {
            JsonValue::Array(mut items)
                if items.len() == 1 && items.first().is_some_and(JsonValue::is_array) =>
            {
                items.remove(0).as_array().cloned().unwrap_or_default()
            }
            JsonValue::Array(items) => items,
            value @ JsonValue::Object(_) => vec![value],
            value => {
                return Err(IcaError::InvalidGroupMembersResponse(format!(
                    "ACK 返回了非成员列表: {value}"
                )));
            }
        }
    } else {
        values
    };

    serde_json::from_value(JsonValue::Array(members)).map_err(|error| {
        IcaError::InvalidGroupMembersResponse(format!("成员字段不符合 Bridge 契约: {error}"))
    })
}

#[cfg(test)]
mod group_member_tests {
    use rust_socketio::Payload;
    use serde_json::json;

    use super::parse_group_members_ack;

    #[test]
    fn parses_supported_ack_shapes_and_rejects_errors() {
        assert!(parse_group_members_ack(Payload::Text(vec![json!([])])).unwrap().is_empty());

        let member = json!({"user_id": 42, "nickname": "member"});
        let single = parse_group_members_ack(Payload::Text(vec![member.clone()])).unwrap();
        assert_eq!(single[0].user_id, 42);

        let wrapped =
            parse_group_members_ack(Payload::Text(vec![json!([[member.clone()]])])).unwrap();
        assert_eq!(wrapped[0].nickname, "member");

        assert!(parse_group_members_ack(Payload::Text(vec![json!({"error": "failed"})])).is_err());
    }

    #[test]
    fn poke_parameters_follow_oicq_conversation_rules() {
        assert_eq!(super::poke_target(-30, 20), Some((30, 20)));
        assert_eq!(super::poke_target(30, 20), Some((30, 30)));
        for (room_id, target) in [(0, 20), (i64::MIN, 20), (-30, 0), (30, -1)] {
            assert_eq!(super::poke_target(room_id, target), None);
        }
    }

    #[test]
    fn auth_challenge_accepts_legacy_versionless_payload_and_rejects_invalid_data() {
        let (salt, version) = super::parse_auth_challenge(&[json!("aabb")]).unwrap();
        assert_eq!(salt, [0xaa, 0xbb]);
        assert!(version.is_null());
        assert!(super::parse_auth_challenge(&[]).is_err());
        assert!(super::parse_auth_challenge(&[json!(17)]).is_err());
        assert!(super::parse_auth_challenge(&[json!("invalid")]).is_err());
        assert!(super::parse_auth_challenge(&[json!("")]).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn token_retry_stops_after_success_and_keeps_the_existing_retry_budget() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let attempts = Arc::new(AtomicUsize::new(0));
        let started = tokio::time::Instant::now();
        let result = super::retry_send_token({
            let attempts = attempts.clone();
            move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    if attempt < 3 {
                        Err(format!("failed-{attempt}"))
                    } else {
                        Ok("token".into())
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(result, "token");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(started.elapsed(), std::time::Duration::from_millis(400));

        attempts.store(0, Ordering::SeqCst);
        let error = super::retry_send_token({
            let attempts = attempts.clone();
            move || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Err(format!("failed-{attempt}")) }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(error, "failed-3");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    fn read_request(
        reader: &mut std::io::BufReader<std::net::TcpStream>,
    ) -> std::io::Result<String> {
        use std::io::{BufRead, Read};
        let mut request = String::new();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line.is_empty() {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            if line == "\r\n" {
                break;
            }
            if request.is_empty() {
                request = line.trim_end().to_owned();
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body)?;
        Ok(request)
    }

    fn respond(stream: &mut std::net::TcpStream, status: &str, extra: &str, close: bool) {
        use std::io::Write;
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: 0\r\n{extra}Connection: {}\r\n\r\n",
            if close { "close" } else { "keep-alive" }
        )
        .unwrap();
        stream.flush().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_sender_reuses_connections_and_only_accepts_202() {
        use std::{
            io::BufReader,
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
            time::Duration,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let accepted_worker = accepted.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            accepted_worker.fetch_add(1, Ordering::SeqCst);
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            assert!(
                read_request(&mut reader).unwrap().starts_with("POST /api/fixture/sendMessage ")
            );
            respond(&mut stream, "202 Accepted", "", false);
            assert!(
                read_request(&mut reader).unwrap().starts_with("POST /api/fixture/sendMessage ")
            );
            respond(&mut stream, "202 Accepted", "", true);
        });
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let context = crate::context::BotContext::new_with_http_client(
            crate::config::BotConfig::default(),
            tokio::runtime::Handle::current(),
            http,
        )
        .unwrap();
        let base = format!("http://{address}");
        super::http_send_message(&base, "fixture", &json!({"content":"one"}), &context)
            .await
            .unwrap();
        super::http_send_message(&base, "fixture", &json!({"content":"two"}), &context)
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }

    async fn rejected_post_is_not_retried_or_redirected(status: &'static str, redirect: bool) {
        use std::{
            io::BufReader,
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
            time::{Duration, Instant},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_worker = requests.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut first = true;
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        requests_worker.fetch_add(1, Ordering::SeqCst);
                        stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let _ = read_request(&mut reader).unwrap();
                        if first {
                            first = false;
                            let location = if redirect {
                                format!("Location: http://{address}/redirected\r\n")
                            } else {
                                String::new()
                            };
                            respond(&mut stream, status, &location, true);
                        } else {
                            respond(&mut stream, "202 Accepted", "", true);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("测试服务器失败: {error}"),
                }
            }
        });
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let context = crate::context::BotContext::new_with_http_client(
            crate::config::BotConfig::default(),
            tokio::runtime::Handle::current(),
            http,
        )
        .unwrap();
        let token = "secret-fixture-token";
        let error = super::http_send_message(
            &format!("http://{address}"),
            token,
            &json!({"content":"one"}),
            &context,
        )
        .await
        .unwrap_err();
        assert!(!error.contains(token));
        server.join().unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_post_does_not_follow_redirects_or_retry_server_errors() {
        rejected_post_is_not_retried_or_redirected("307 Temporary Redirect", true).await;
        rejected_post_is_not_retried_or_redirected("500 Internal Server Error", false).await;
    }
}
