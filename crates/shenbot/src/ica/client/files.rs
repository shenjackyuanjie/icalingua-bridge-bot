//! 普通文件上传与群文件管理的 oicq 业务管线。
//!
//! 主连接使用固定 512 KiB 分片及 JSON 字节数组，Bridge 需允许至少 `2 MiB + 256 B`
//! 的 Socket.IO 载荷（建议配置 4 MiB）。群目录操作使用 token 鉴权的 fileMgr 短连接，
//! 不把它们发到主连接，也不把文件消息的传输受理当作 QQ 上传成功。

#[cfg(test)]
#[path = "tests/files.rs"]
mod tests;

use super::bridge;
use crate::context::BotContext;
use futures_util::FutureExt;
use rust_socketio::{
    Payload, TransportType,
    asynchronous::{Client, ClientBuilder},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

pub const FILE_CHUNK_SIZE: usize = 512 * 1024;
const FILE_ACK_TIMEOUT: Duration = Duration::from_secs(30);
const GROUP_UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const FILE_AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const CHUNK_ATTEMPTS: usize = 3;

struct UploadStatus {
    complete: bool,
    uploaded: BTreeSet<usize>,
}

fn sha256(data: &[u8]) -> String { hex::encode(Sha256::digest(data)) }

fn upload_status(value: Value, file_size: usize) -> Result<UploadStatus, String> {
    let complete = value
        .get("allSuccess")
        .and_then(Value::as_bool)
        .ok_or_else(|| "requestUpload ACK 缺少 allSuccess 布尔值".to_string())?;
    let offsets = value
        .get("uploaded")
        .and_then(Value::as_array)
        .ok_or_else(|| "requestUpload ACK 缺少 uploaded 偏移列表".to_string())?;
    let mut uploaded = BTreeSet::new();
    for offset in offsets {
        let offset = offset
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "requestUpload ACK 包含非整数偏移".to_string())?;
        if offset >= file_size || !offset.is_multiple_of(FILE_CHUNK_SIZE) {
            return Err("requestUpload ACK 包含越界或未对齐的分片偏移".into());
        }
        uploaded.insert(offset);
    }
    if complete && uploaded.len() != file_size.div_ceil(FILE_CHUNK_SIZE) {
        return Err("requestUpload ACK 声称完成但分片列表不完整".into());
    }
    Ok(UploadStatus { complete, uploaded })
}

fn validate_file_name(file_name: &str) -> Result<(), String> {
    if file_name.trim().is_empty() {
        return Err("文件名不能为空".into());
    }
    Ok(())
}

/// 上传并验证完整文件，返回 Bridge 临时文件的 SHA-256 引用。
///
/// 已有正确分片会跳过；单片写入可幂等重试。返回前再次 requestUpload，让服务端校验
/// 整文件 SHA-256；上传后的发送/入群操作由调用者单独执行，不自动重试这些业务修改。
pub async fn upload_file_to_bridge(
    client: &Client,
    file_name: &str,
    data: &[u8],
) -> Result<String, String> {
    upload_file_with(file_name, data, |event, args, timeout| {
        bridge::request_event(client, event, args, timeout)
    })
    .await
}

async fn upload_file_with<R, F>(
    file_name: &str,
    data: &[u8],
    mut request: R,
) -> Result<String, String>
where
    R: FnMut(&'static str, Vec<Value>, Duration) -> F,
    F: Future<Output = Result<Value, String>>,
{
    validate_file_name(file_name)?;
    let file_hash = sha256(data);
    let upload_args = vec![json!(file_name), json!(file_hash), json!(data.len())];
    let status = upload_status(
        bridge::checked_response(
            "requestUpload",
            request("requestUpload", upload_args.clone(), FILE_ACK_TIMEOUT).await?,
        )?,
        data.len(),
    )?;
    if status.complete {
        return Ok(file_hash);
    }

    // 分片数齐全而整文件校验失败时，不能把这些偏移视为已验证；覆盖全部分片修复损坏数据。
    let overwrite_all = status.uploaded.len() == data.len().div_ceil(FILE_CHUNK_SIZE);
    for (index, chunk) in data.chunks(FILE_CHUNK_SIZE).enumerate() {
        let offset = index * FILE_CHUNK_SIZE;
        if !overwrite_all && status.uploaded.contains(&offset) {
            continue;
        }
        let args = vec![json!(file_hash), json!(offset), json!(chunk), json!(sha256(chunk))];
        let mut last_error = "uploadFile 未确认分片写入".to_string();
        let mut accepted = false;
        for attempt in 0..CHUNK_ATTEMPTS {
            let response = request("uploadFile", args.clone(), FILE_ACK_TIMEOUT)
                .await
                .and_then(|value| bridge::checked_response("uploadFile", value));
            match response {
                Ok(Value::Bool(true)) => {
                    accepted = true;
                    break;
                }
                Ok(_) => last_error = "uploadFile ACK 不是成功布尔值".into(),
                Err(error) => last_error = error,
            }
            if attempt + 1 < CHUNK_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        if !accepted {
            return Err(format!("文件分片偏移 {offset} 上传失败: {last_error}"));
        }
    }
    let verified = upload_status(
        bridge::checked_response(
            "requestUpload",
            request("requestUpload", upload_args, FILE_ACK_TIMEOUT).await?,
        )?,
        data.len(),
    )?;
    if !verified.complete {
        return Err("文件分片已发送，但 Bridge 未确认整文件 SHA-256 校验成功".into());
    }
    Ok(file_hash)
}

fn ordinary_file_mime(mime: &str) -> Result<&str, String> {
    // 该 API 使用临时文件 hash，而媒体 API 使用 URL/base64/fid，两种路径明确分开。
    if mime.contains("image") || mime.starts_with("audio") {
        return Err("普通文件 API 不接收图片/音频 MIME，请使用媒体发送接口；作为文档发送时明确使用 application/octet-stream".into());
    }
    Ok(if mime.is_empty() {
        "application/octet-stream"
    } else {
        mime
    })
}

fn file_message(
    room: i64,
    content: &str,
    reply: Option<Value>,
    at: Vec<Value>,
    mime: &str,
    file_hash: &str,
    file_size: usize,
) -> Result<Value, String> {
    if room == 0 {
        return Err("文件消息需要非零房间 ID".into());
    }
    let mime = ordinary_file_mime(mime)?;
    let mut message = json!({
        "roomId": room,
        "content": content,
        "at": at,
        "messageType": "text",
        "file": {"type": mime, "path": file_hash, "size": file_size},
    });
    if let Some(reply) = reply {
        message["replyMessage"] = reply;
    }
    Ok(message)
}

/// 上传本地字节，再向私聊或群聊发送普通文件；成功结果仅表示发送受理。
///
/// 参数完整交给 Bridge；其普通文件分支不构造回复/@消息链，content 只可能用于私聊回显，
/// 不表示 QQ 文件会附带这些正文或引用信息。图片/音频请使用媒体接口，普通文档可明确使用 octet-stream。
#[allow(clippy::too_many_arguments)]
pub async fn upload_and_send_file(
    client: &Client,
    room: i64,
    content: &str,
    reply: Option<Value>,
    at: Vec<Value>,
    file_name: &str,
    mime: &str,
    data: &[u8],
) -> Result<Value, String> {
    if room == 0 {
        return Err("文件消息需要非零房间 ID".into());
    }
    let mime = ordinary_file_mime(mime)?;
    let file_hash = upload_file_to_bridge(client, file_name, data).await?;
    let message = file_message(room, content, reply, at, mime, &file_hash, data.len())?;
    bridge::emit_event(client, "sendMessage", vec![message]).await
}

/// 主连接的群文件上传有真实 ACK；失败后 Bridge 会清除临时文件，故不自动重试最终提交。
pub async fn upload_group_file(
    client: &Client,
    gid: i64,
    parent: &str,
    file_name: &str,
    data: &[u8],
) -> Result<Value, String> {
    if gid <= 0 {
        return Err("群文件 API 使用正群号，而不是负 roomId".into());
    }
    let file_hash = upload_file_to_bridge(client, file_name, data).await?;
    let args = group_upload_arguments(&file_hash, gid, parent, file_name);
    let response =
        bridge::request_event(client, "uploadGroupFile", args, GROUP_UPLOAD_TIMEOUT).await?;
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("uploadGroupFile ACK 未确认上传成功".into());
    }
    Ok(response)
}

fn group_upload_arguments(hash: &str, gid: i64, parent: &str, name: &str) -> Vec<Value> {
    vec![
        json!(hash),
        json!(gid),
        json!(if parent.is_empty() { "/" } else { parent }),
        json!(name),
    ]
}

fn validate_file_manager(event: &str, args: &[Value]) -> Result<(), String> {
    let count = bridge::file_manager_argument_count(event)
        .ok_or_else(|| format!("{event} 不是当前 oicq fileMgr 事件"))?;
    if args.len() != count {
        return Err(format!("fileMgr {event} 需要 {count} 个位置参数，收到 {} 个", args.len()));
    }
    for (index, value) in args.iter().enumerate() {
        if event == "ls" && index == 1 {
            if value.as_u64().is_none() {
                return Err("fileMgr ls 的 start 必须是非负整数".into());
            }
        } else if value.as_str().is_none() {
            return Err(format!("fileMgr {event} 的第 {} 个参数必须是字符串", index + 1));
        }
    }
    Ok(())
}

fn file_authentication(payload: Payload, expected_gid: i64) -> Result<(), String> {
    // 普通事件回调已是 [gin, group]，不同于 ACK 外层包装，不能用 ACK 的数组展开规则。
    let Payload::Text(values) = payload else {
        return Err("fileMgr 鉴权回包不是位置参数列表".into());
    };
    if values.first().and_then(Value::as_i64) != Some(expected_gid) {
        return Err("fileMgr 鉴权回包的群号与请求不一致".into());
    }
    Ok(())
}

type AuthSender = oneshot::Sender<Result<(), String>>;

#[derive(Clone)]
struct AuthResponder(Arc<Mutex<Option<AuthSender>>>);

impl AuthResponder {
    fn respond(&self, result: Result<(), String>) {
        if let Ok(mut sender) = self.0.lock()
            && let Some(sender) = sender.take()
        {
            let _ = sender.send(result);
        }
    }
}

/// 取消业务 future 时仍清理已建立的专用连接；不保留 token 到日志或全局状态。
struct FileManagerConnection(Option<Client>);

impl FileManagerConnection {
    async fn close(&mut self) {
        if let Some(client) = &self.0 {
            match tokio::time::timeout(CLEANUP_TIMEOUT, client.disconnect()).await {
                Ok(Ok(())) => {
                    self.0.take();
                }
                _ => tracing::warn!("fileMgr 连接清理未完成，将执行后台有界清理"),
            }
        }
    }
}

impl Drop for FileManagerConnection {
    fn drop(&mut self) {
        if let Some(client) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(CLEANUP_TIMEOUT, client.disconnect()).await;
            });
        }
    }
}

async fn after_authentication<R, F>(
    receiver: oneshot::Receiver<Result<(), String>>,
    auth_timeout: Duration,
    request: R,
) -> Result<Value, String>
where
    R: FnOnce() -> F,
    F: Future<Output = Result<Value, String>>,
{
    tokio::time::timeout(auth_timeout, receiver)
        .await
        .map_err(|_| "fileMgr 鉴权超时".to_string())?
        .map_err(|_| "fileMgr 鉴权通道提前关闭".to_string())??;
    request().await
}

/// 一次 `requestGfsToken -> 建连 -> auth(token,"fileMgr") -> 业务 ACK -> 断开`。
///
/// timeout 是业务 ACK 上界；取 token、建连、鉴权均另有不超过该值的短上界。
/// fileMgr 的 mkdir 合同仅接收 name；ls 的空数组可能来自空目录，也可能是 Bridge 捕获错误。
pub async fn file_manager_request(
    main_client: &Client,
    context: &BotContext,
    gid: i64,
    event: &str,
    args: Vec<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    if gid <= 0 {
        return Err("群文件 API 使用正群号，而不是负 roomId".into());
    }
    if timeout.is_zero() {
        return Err("fileMgr 超时必须大于零".into());
    }
    validate_file_manager(event, &args)?;
    let token = bridge::request_event(
        main_client,
        "requestGfsToken",
        vec![json!(gid)],
        timeout.min(FILE_ACK_TIMEOUT),
    )
    .await?;
    let token = token
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "requestGfsToken ACK 未提供非空 token".to_string())?
        .to_owned();

    let (sender, receiver) = oneshot::channel();
    let responder = AuthResponder(Arc::new(Mutex::new(Some(sender))));
    let auth_timeout = timeout.min(FILE_AUTH_TIMEOUT);
    let auth_sender = responder.clone();
    let success_sender = responder.clone();
    let failure_sender = responder.clone();
    let error_sender = responder.clone();
    let close_sender = responder;
    let builder = ClientBuilder::new(context.config.ica().host.clone())
        .transport_type(TransportType::Websocket)
        .reconnect(false)
        .reconnect_on_disconnect(false)
        .on("requireAuth", move |_, client| {
            let token = token.clone();
            let responder = auth_sender.clone();
            async move {
                let result = tokio::time::timeout(
                    auth_timeout,
                    client.emit("auth", vec![json!(token), json!("fileMgr")]),
                )
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    responder.respond(Err("fileMgr 认证事件发送失败或超时".into()));
                }
            }
            .boxed()
        })
        .on("authSucceed", move |payload, _| {
            let responder = success_sender.clone();
            async move {
                responder.respond(file_authentication(payload, gid));
            }
            .boxed()
        })
        .on("authFailed", move |_, _| {
            let responder = failure_sender.clone();
            async move {
                responder.respond(Err("fileMgr 鉴权失败".into()));
            }
            .boxed()
        })
        .on("error", move |_, _| {
            let responder = error_sender.clone();
            async move {
                responder.respond(Err("fileMgr 连接发生错误".into()));
            }
            .boxed()
        })
        .on("close", move |_, _| {
            let responder = close_sender.clone();
            async move {
                responder.respond(Err("fileMgr 连接提前关闭".into()));
            }
            .boxed()
        });
    let client = tokio::time::timeout(auth_timeout, builder.connect())
        .await
        .map_err(|_| "fileMgr 建连超时".to_string())?
        .map_err(|error| format!("fileMgr 建连失败: {error}"))?;
    let mut connection = FileManagerConnection(Some(client.clone()));
    let result = after_authentication(receiver, auth_timeout, || {
        bridge::request_unchecked(&client, event, args, timeout)
    })
    .await;
    connection.close().await;
    result
}
