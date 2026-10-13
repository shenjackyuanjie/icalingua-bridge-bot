//! Socket.IO/oicq Bridge 的位置参数、真实 ACK 和可消费查询。
//!
//! 查询保留完整 Bridge JSON；发送受理与 QQ 业务成功分别表示，修改类事件不等待伪 ACK。

#[cfg(test)]
#[path = "tests/bridge.rs"]
mod tests;

use super::{ack, ack_payload_values};
use rust_socketio::{Payload, asynchronous::Client};
use serde_json::{Value, json};
use std::time::Duration;

const EMIT_TIMEOUT: Duration = Duration::from_secs(30);

/// 仅表示位置参数的数量，不对未知扩展事件施加白名单。
fn ack_argument_count(event: &str) -> Option<usize> {
    Some(match event {
        "validateMessageSearchIndex"
        | "migrateLegacyAtMessages"
        | "getGroups"
        | "getIgnoredChats"
        | "getFriendsFallback"
        | "getSystemMsg"
        | "requestToken"
        | "getDisabledFeatures"
        | "getLoginDevices" => 0,
        "getFirstUnreadRoom" | "getRoom" | "getUnreadCount" | "getCookies" | "getRoamingStamp"
        | "getGroupMembers" | "requestGfsToken" | "getFriend" | "getGroup" | "getMsgNewURL" => 1,
        "fetchMessages"
        | "resolveUnreadTargetMessageId"
        | "markMessageUnread"
        | "getForwardMsg"
        | "getGroupFileMeta"
        | "getNTPicURLbyFileid" => 2,
        "fetchImageMessages" | "getGroupMemberInfo" | "sendPacket" | "requestUpload" => 3,
        "fetchMessagesAround" | "fetchMessagesBySender" | "uploadFile" | "uploadGroupFile" => 4,
        "searchMessages" => 6,
        _ => return None,
    })
}

fn is_mutation_without_ack(event: &str) -> bool {
    matches!(
        event,
        "addRoom"
            | "addChatGroup"
            | "updateChatGroup"
            | "removeChatGroup"
            | "deleteMessage"
            | "hideMessage"
            | "fetchHistory"
            | "fetch7DaysHistory"
            | "ignoreChat"
            | "pinRoom"
            | "reLogin"
            | "removeChat"
            | "revealMessage"
            | "renewMessage"
            | "renewMessageURL"
            | "sendGroupPoke"
            | "sendGroupSign"
            | "sendMessage"
            | "setOnlineStatus"
            | "setRoomAutoDownload"
            | "setRoomAutoDownloadPath"
            | "setRoomPriority"
            | "updateMessage"
            | "updateRoom"
            | "removeIgnoredChat"
            | "setGroupNick"
            | "handleRequest"
            | "setGroupLeave"
            | "setGroupKick"
            | "setGroupBan"
            | "setGroupAnonymousBan"
            | "sendButtonCallback"
            | "setGroupRemark"
            | "setFriendRemark"
            | "makeForward"
            | "login"
            | "submitSmsCode"
            | "randomDevice"
            | "login-verify-reLogin"
            | "login-slider-ticket"
            | "deleteLoginDevice"
    )
}

pub fn file_manager_argument_count(event: &str) -> Option<usize> {
    match event {
        "ls" | "mv" | "rename" => Some(2),
        "download" | "stat" | "mkdir" | "rm" => Some(1),
        _ => None,
    }
}

fn validate_main_event(event: &str, args: &[Value], expect_ack: bool) -> Result<(), String> {
    if event.trim().is_empty() {
        return Err("Socket 事件名不能为空".into());
    }
    if file_manager_argument_count(event).is_some() {
        return Err(format!("{event} 属于 fileMgr 连接，请使用群文件管理 API"));
    }
    if matches!(event, "reportRead" | "stopFetchingHistory" | "preloadImages") {
        return Err(format!("当前 oicq Bridge 未注册 {event} 事件"));
    }
    if event == "getPrivateFileUrl" {
        return Err("当前 oicq Bridge 尚未实现 getPrivateFileUrl".into());
    }
    if expect_ack && is_mutation_without_ack(event) {
        return Err(format!("{event} 不提供 ACK，请使用 emit_event；发送受理不等于 QQ 操作成功"));
    }
    if let Some(count) = ack_argument_count(event)
        && args.len() != count
    {
        return Err(format!(
            "{event} 需要 {count} 个位置参数，收到 {} 个；可选位置请填 null",
            args.len()
        ));
    }
    Ok(())
}

/// 从真实 ACK 的参数包装中取回业务值，只展开协议层一次。
///
/// `cb(messages)` 是一个数组值，而不是多个消息位置参数；扩展事件的多个 ACK 实参仍返回数组。
pub fn decode_ack(payload: Payload) -> Value {
    let mut values = ack_payload_values(payload);
    match values.len() {
        0 => Value::Null,
        1 => values.remove(0),
        _ => Value::Array(values),
    }
}

/// oicq 以几种旧式错误形状回报，统一转换为显式错误而不丢弃成功结果。
pub fn checked_response(event: &str, value: Value) -> Result<Value, String> {
    if let Some(error) = value.get("error")
        && !error.is_null()
        && error != &Value::Bool(false)
    {
        let detail = error.as_str().map(str::to_owned).unwrap_or_else(|| error.to_string());
        return Err(format!("{event} 失败: {detail}"));
    }
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(format!("{event} 返回失败 ACK"));
    }
    if value.as_str() == Some("error") || value.get("url").and_then(Value::as_str) == Some("error")
    {
        let detail = value.get("name").and_then(Value::as_str).unwrap_or("资源地址获取失败");
        return Err(format!("{event} 失败: {detail}"));
    }
    if event == "uploadFile" && value.as_bool() == Some(false) {
        return Err("uploadFile 返回失败 ACK".into());
    }
    Ok(value)
}

/// 发出主连接事件，返回 `{"sent":true}` 仅表示本地传输受理。
///
/// 未知扩展事件保留可调用性；已知不可达事件、fileMgr 事件则在发送前给出明确错误。
pub async fn emit_event(client: &Client, event: &str, args: Vec<Value>) -> Result<Value, String> {
    validate_main_event(event, &args, false)?;
    tokio::time::timeout(EMIT_TIMEOUT, client.emit(event, args))
        .await
        .map_err(|_| format!("{event} 发送超时"))?
        .map_err(|error| format!("{event} 发送失败: {error}"))?;
    Ok(json!({"sent": true}))
}

/// 请求真实 ACK，截止时间同时覆盖注册和实际回包等待。
///
/// `args` 的每项是一个 Socket.IO 位置参数，数组业务参数不会再包装或展开。
pub async fn request_event(
    client: &Client,
    event: &str,
    args: Vec<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    validate_main_event(event, &args, true)?;
    if timeout.is_zero() {
        return Err("ACK 超时必须大于零".into());
    }
    request_unchecked(client, event, args, timeout).await
}

/// 调用者已确定所在命名空间；用于 fileMgr 的独立短连接。
pub async fn request_unchecked(
    client: &Client,
    event: &str,
    args: Vec<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    let payload = ack::request(client, event, args.into(), timeout)
        .await
        .map_err(|error| format!("{event}: {error}"))?;
    checked_response(event, decode_ack(payload))
}

fn array_result(event: &str, value: Value) -> Result<Vec<Value>, String> {
    match value {
        Value::Array(values) => Ok(values),
        _ => Err(format!("{event} ACK 不是列表")),
    }
}

fn message_result(event: &str, value: Value) -> Result<Vec<Value>, String> {
    let messages = array_result(event, value)?;
    if messages.iter().any(|value| !value.is_object() || value.get("_id").is_none()) {
        return Err(format!("{event} ACK 包含无效消息对象"));
    }
    Ok(messages)
}

fn validate_page_options(options: Value) -> Result<Value, String> {
    let options = if options.is_null() {
        json!({})
    } else {
        options
    };
    let Some(object) = options.as_object() else {
        return Err("fetchMessages options 必须是对象".into());
    };
    let has_before = object.get("before").is_some_and(|value| !value.is_null());
    let has_after = object.get("after").is_some_and(|value| !value.is_null());
    if has_before && has_after {
        return Err("fetchMessages 的 before 和 after 游标不能同时提供".into());
    }
    for name in ["before", "after"] {
        if let Some(cursor) = object.get(name).filter(|value| !value.is_null())
            && (!cursor.get("time").is_some_and(Value::is_number)
                || !cursor.get("id").is_some_and(|id| id.is_string() || id.is_number()))
        {
            return Err(format!("{name} 游标需要数值 time 及字符串或数值 id"));
        }
    }
    Ok(options)
}

fn search_arguments(
    room: i64,
    keyword: &str,
    offset: usize,
    sender_id: Option<i64>,
    start_time: Option<i64>,
    end_time: Option<i64>,
) -> Vec<Value> {
    vec![
        json!(room),
        json!(keyword),
        json!(offset),
        json!(sender_id),
        json!(start_time),
        json!(end_time),
    ]
}

pub async fn fetch_messages(
    client: &Client,
    room: i64,
    options: Value,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let options = validate_page_options(options)?;
    let value = request_event(client, "fetchMessages", vec![json!(room), options], timeout).await?;
    message_result("fetchMessages", value)
}

/// `room=0` 可搜索本地存储中的所有房间；返回保留 roomId 的完整消息。
#[allow(clippy::too_many_arguments)]
pub async fn search_messages(
    client: &Client,
    room: i64,
    keyword: &str,
    offset: usize,
    sender_id: Option<i64>,
    start_time: Option<i64>,
    end_time: Option<i64>,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let args = search_arguments(room, keyword, offset, sender_id, start_time, end_time);
    let value = request_event(client, "searchMessages", args, timeout).await?;
    message_result("searchMessages", value)
}

pub async fn fetch_messages_by_sender(
    client: &Client,
    room: i64,
    sender: i64,
    offset: usize,
    snapshot_time: Option<i64>,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let args = vec![json!(room), json!(sender), json!(offset), json!(snapshot_time)];
    let value = request_event(client, "fetchMessagesBySender", args, timeout).await?;
    message_result("fetchMessagesBySender", value)
}

pub async fn fetch_messages_around(
    client: &Client,
    room: i64,
    message_id: &str,
    before: usize,
    after: usize,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let args = vec![json!(room), json!(message_id), json!(before), json!(after)];
    let value = request_event(client, "fetchMessagesAround", args, timeout).await?;
    message_result("fetchMessagesAround", value)
}

pub async fn fetch_image_messages(
    client: &Client,
    room: i64,
    offset: usize,
    end_time: Option<i64>,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let args = vec![json!(room), json!(offset), json!(end_time)];
    let value = request_event(client, "fetchImageMessages", args, timeout).await?;
    message_result("fetchImageMessages", value)
}

pub async fn get_system_messages(client: &Client, timeout: Duration) -> Result<Value, String> {
    let value = request_event(client, "getSystemMsg", vec![], timeout).await?;
    if !value.is_object() {
        return Err("getSystemMsg ACK 不是申请字典".into());
    }
    Ok(value)
}

pub async fn get_friends(client: &Client, timeout: Duration) -> Result<Vec<Value>, String> {
    let value = request_event(client, "getFriendsFallback", vec![], timeout).await?;
    array_result("getFriendsFallback", value)
}

pub async fn get_groups(client: &Client, timeout: Duration) -> Result<Vec<Value>, String> {
    let value = request_event(client, "getGroups", vec![], timeout).await?;
    array_result("getGroups", value)
}

fn forward_result(value: Value) -> Result<Vec<Value>, String> {
    let messages = message_result("getForwardMsg", value)?;
    // oicq 将资源读取失败编码成这一个系统占位；不误判昵称为“错误”的正常发言。
    if messages.len() == 1 {
        let message = &messages[0];
        if message.get("senderId").and_then(Value::as_i64) == Some(0)
            && message.get("username").and_then(Value::as_str) == Some("错误")
            && message.get("_id").and_then(Value::as_i64) == Some(0)
            && message.get("time").and_then(Value::as_i64) == Some(0)
        {
            let detail = message.get("content").and_then(Value::as_str).unwrap_or("资源读取失败");
            return Err(format!("getForwardMsg 失败: {detail}"));
        }
    }
    Ok(messages)
}

pub async fn get_forward_messages(
    client: &Client,
    res_id: &str,
    file_name: Option<&str>,
    timeout: Duration,
) -> Result<Vec<Value>, String> {
    let args = vec![json!(res_id), json!(file_name.unwrap_or("MultiMsg"))];
    let value = request_event(client, "getForwardMsg", args, timeout).await?;
    forward_result(value)
}
