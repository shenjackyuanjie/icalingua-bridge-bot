//! Icalingua bridge 主动推送事件和 ACK 响应的处理函数。

#[cfg(test)]
#[path = "tests/events.rs"]
mod tests;

use colored::Colorize;
use rust_socketio::asynchronous::Client;
use rust_socketio::{Event, Payload};
use serde_json::{Value as JsonValue, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{Level, event, info, span};

use crate::commands::{self, Reply, Source};
use crate::data_struct::ica::RoomId;
use crate::data_struct::ica::all_rooms::JoinRequestRoom;
use crate::data_struct::ica::messages::{MessageTrait, NewMessage};
use crate::data_struct::ica::online_data::OnlineData;
use crate::host::BotHost;
use crate::ica::client::{bridge, send_message};
use crate::ica::connection::ConnectionState;
use crate::ica::event_state;

fn first_arg(payload: &Payload) -> Option<&JsonValue> {
    match payload {
        Payload::Text(values) => values.first(),
        _ => None,
    }
}

fn warn_invalid(host: &BotHost, event_name: &'static str, error: Option<&serde_json::Error>) {
    let rejected = host
        .context
        .update_ica(|status| status.event_diagnostics.record_invalid(event_name, Instant::now()));
    if let Some(rejected) = rejected {
        event!(Level::WARN, event = event_name,
            category = ?error.map(serde_json::Error::classify), rejected,
            "Bridge 事件格式无效，已跳过；同类事件每 30 秒最多提示一次，不记录原始内容");
    } else {
        event!(Level::DEBUG, event = event_name,
            category = ?error.map(serde_json::Error::classify),
            "Bridge 无效事件已跳过，重复警告已合并");
    }
}

/// 保持 Socket.IO 参数位置；一个数组参数不展开成多个参数。
fn payload_args(payload: &Payload) -> Vec<JsonValue> {
    match payload {
        Payload::Text(values) => values.clone(),
        Payload::Binary(bytes) => vec![json!(bytes.to_vec())],
        _ => Vec::new(),
    }
}

pub async fn bridge_event(
    payload: &Payload,
    client: &Client,
    host: &Arc<BotHost>,
    event_name: &str,
) {
    host.dispatch_ica_bridge_event(client, event_name, || payload_args(payload))
        .await;
}

/// 获取在线数据
pub async fn get_online_data(payload: Payload, client: Client, state: (Arc<BotHost>, u64)) {
    let (host, generation) = state;
    if let Some(value) = first_arg(&payload) {
        // 独立 oicq Bridge 只向 authed 房间发送 onlineData；无可靠账号不声明可发送。
        host.ica_connection().online_data(
            generation,
            client.clone(),
            value.get("uin").and_then(JsonValue::as_i64),
            value.get("online").and_then(JsonValue::as_bool).unwrap_or(false),
        );
        let online_data = OnlineData::new_from_json(value);
        event!(Level::DEBUG, online = online_data.online, "更新 Bridge 在线状态");
        host.context.update_ica(|status| {
            status.qq_login = online_data.online;
            status.update_online_status(online_data);
        });
    }
    bridge_event(&payload, &client, &host, "onlineData").await;
}

/// 处理 `setOnline`，把本地 QQ 登录状态标记为在线。
pub async fn set_online(payload: Payload, client: Client, state: (Arc<BotHost>, u64)) {
    let (host, generation) = state;
    host.ica_connection().set_online(generation, true);
    host.context.update_ica(|status| status.qq_login = true);
    event!(Level::INFO, "Icalingua 已上线");
    bridge_event(&payload, &client, &host, "setOnline").await;
}

/// 处理 `setOffline`，把本地 QQ 登录状态标记为离线并记录原因。
pub async fn set_offline(payload: Payload, client: Client, state: (Arc<BotHost>, u64)) {
    let (host, generation) = state;
    host.ica_connection().set_online(generation, false);
    host.context.update_ica(|status| status.qq_login = false);
    event!(Level::WARN, "Icalingua 已离线，原因已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "setOffline").await;
}

/// 处理 `setShutUp`，记录当前会话的禁言状态变化。
pub async fn set_shut_up(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::INFO, shut_up = ?first_arg(&payload).and_then(JsonValue::as_bool), "当前会话禁言状态变化");
    bridge_event(&payload, &client, &host, "setShutUp").await;
}

/// Python 的传输候选保持在 sender/quote/self 和普通命令过滤之前。
pub(crate) async fn before_message_filters<T: Clone>(
    host: &BotHost,
    connection: &ConnectionState<T>,
    generation: u64,
    message: &NewMessage,
) -> bool {
    host.dispatch_ica_candidate(connection, generation, message).await;
    host.context.config.ica().filter_list.contains(&message.msg.sender_id)
}

pub async fn add_message(payload: Payload, client: Client, state: (Arc<BotHost>, u64)) {
    let (host, generation) = state;
    let Some(value) = first_arg(&payload) else {
        warn_invalid(&host, "addMessage", None);
        return;
    };
    let message = match event_state::decode_add_message(value) {
        Ok(message) => message,
        Err(error) => {
            warn_invalid(&host, "addMessage", Some(&error));
            return;
        }
    };
    let connection = host.ica_connection().current();
    if !connection.open
        || connection.generation != generation
        || before_message_filters(&host, &connection, generation, &message).await
    {
        return;
    }
    println!("new_msg {}", message.to_string().cyan());
    if host
        .ica_connection()
        .with_open_generation(generation, || {
            host.context.update_ica(|status| {
                status.store_loaded_messages(message.room_id, vec![message.msg.clone()], false);
            });
        })
        .is_none()
    {
        return;
    }
    let command = commands::parse(
        message.content(),
        Source::Ica {
            from_self: host
                .context
                .read_ica(|status| message.is_from_self(status.online_status.qqid)),
        },
        host.context.client_id(),
        message.is_reply(),
        host.context.config.ica().admin_list.contains(&message.sender_id()),
    );
    let worker = host.clone();
    commands::dispatch(&host, command, move |reply| async move {
        let host = worker;
        if host.ica_connection().with_open_generation(generation, || ()).is_none() {
            return;
        }
        match reply {
            Some(Reply::Text(text)) => {
                send_message(&client, &message.reply_with(&text), &host.context).await;
            }
            Some(Reply::FetchRoom) => {
                if let Ok(permit) = host.fetch_slot.clone().try_acquire_owned() {
                    send_message(&client, &message.reply_with("正在更新当前群消息"), &host.context)
                        .await;
                    let (client, host, room) = (client.clone(), host.clone(), message.room_id);
                    tokio::spawn(async move {
                        let _permit = permit;
                        fetch_messages(&client, room, &host, generation).await;
                    });
                } else {
                    send_message(
                        &client,
                        &message.reply_with("消息更新正在进行，请稍后再试"),
                        &host.context,
                    )
                    .await;
                }
            }
            None => {}
        }
        host.dispatch_ica_message(&message, &client, generation).await;
    })
    .await;
}

/// 历史推送只更新有界已加载缓存，不重放普通/系统新消息 hook。
pub async fn set_messages(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::set_messages(status, value)) {
            Ok(batch) => {
                println!(
                    "set_messages {} len: {}",
                    value["roomId"].to_string().cyan(),
                    value["messages"].as_array().map_or(0, Vec::len)
                );
                info!(
                    accepted = batch.accepted,
                    rejected = batch.rejected,
                    cached = batch.cached,
                    "已消费 Bridge 历史消息批次"
                );
                bridge_event(&payload, &client, &host, "setMessages").await;
            }
            Err(error) => warn_invalid(&host, "setMessages", Some(&error)),
        }
    } else {
        warn_invalid(&host, "setMessages", None);
    }
}

/// 直接撤回和真实 QQ renew 撤回共享有界去重及缓存标记。
pub async fn delete_message(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::delete_message(status, value)) {
            Ok(Some(message_id)) => host.dispatch_ica_delete(message_id, &client).await,
            Ok(None) => (),
            Err(error) => warn_invalid(&host, "deleteMessage", Some(&error)),
        }
    } else {
        warn_invalid(&host, "deleteMessage", None);
    }
}

/// 完整房间快照安全解析，同时保留 typed Room 以外的业务字段。
pub async fn update_all_room(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::set_all_rooms(status, value)) {
            Ok(count) => {
                event!(Level::DEBUG, rooms = count, "更新完整房间快照");
                bridge_event(&payload, &client, &host, "setAllRooms").await;
            }
            Err(error) => warn_invalid(&host, "setAllRooms", Some(&error)),
        }
    } else {
        warn_invalid(&host, "setAllRooms", None);
    }
}

/// 单房间补丁先验证再合并，不因畸形字段破坏已知快照。
pub async fn update_room(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::update_room(status, value)) {
            Ok(()) => bridge_event(&payload, &client, &host, "updateRoom").await,
            Err(error) => warn_invalid(&host, "updateRoom", Some(&error)),
        }
    } else {
        warn_invalid(&host, "updateRoom", None);
    }
}

pub async fn update_chat_groups(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host
            .context
            .update_ica(|status| event_state::set_all_chat_groups(status, value))
        {
            Ok(_) => bridge_event(&payload, &client, &host, "setAllChatGroups").await,
            Err(error) => warn_invalid(&host, "setAllChatGroups", Some(&error)),
        }
    } else {
        warn_invalid(&host, "setAllChatGroups", None);
    }
}

pub async fn sync_read(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::sync_read(status, value)) {
            Ok(_) => bridge_event(&payload, &client, &host, "syncRead").await,
            Err(error) => warn_invalid(&host, "syncRead", Some(&error)),
        }
    } else {
        warn_invalid(&host, "syncRead", None);
    }
}

/// 广播提示不是单次发送 ACK；原始结果供插件自行消费，不作为发送成功确认。
pub async fn success_message(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        println!("messageSuccess {}", value.to_string().green());
    }
    bridge_event(&payload, &client, &host, "messageSuccess").await;
}

pub async fn failed_message(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "收到 Bridge 操作失败提示，详情已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "messageError").await;
}

/// 消息补丁更新已加载缓存；QQ 撤回首次映射旧删除 hook，且不重放新消息。
pub async fn renew_message(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::renew_message(status, value)) {
            Ok(update) => {
                if update.first_deletion {
                    host.dispatch_ica_delete(update.message_id.clone(), &client).await;
                }
                host.dispatch_ica_update(
                    &client,
                    update.room_id,
                    &update.message_id,
                    &update.patch,
                )
                .await;
            }
            Err(error) => warn_invalid(&host, "renewMessage", Some(&error)),
        }
    } else {
        warn_invalid(&host, "renewMessage", None);
    }
}

/// URL 事件没有 roomId；从已加载缓存关联真实房间，同时保留原 wire 事件。
pub async fn renew_message_url(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::renew_message_url(status, value)) {
            Ok(updates) => {
                for (room_id, message_id, patch) in updates {
                    host.dispatch_ica_update(&client, room_id, &message_id, &patch).await;
                }
                bridge_event(&payload, &client, &host, "renewMessageURL").await;
            }
            Err(error) => warn_invalid(&host, "renewMessageURL", Some(&error)),
        }
    } else {
        warn_invalid(&host, "renewMessageURL", None);
    }
}

pub async fn notify_error(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "收到 Bridge 错误通知，详情已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "notifyError").await;
}

pub async fn fatal_error(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::ERROR, "收到 Bridge 致命错误通知");
    bridge_event(&payload, &client, &host, "fatal").await;
}

pub async fn request_setup(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "Bridge 尚未完成账号配置；配置字段仅交给业务事件 hook");
    bridge_event(&payload, &client, &host, "requestSetup").await;
}

pub async fn login_verify(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "Bridge 登录需要设备验证；挑战内容不写日志");
    bridge_event(&payload, &client, &host, "login-verify").await;
}

pub async fn login_qrcode(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "Bridge 登录需要扫码；二维码内容不写日志");
    bridge_event(&payload, &client, &host, "login-qrcodeLogin").await;
}

pub async fn login_sms_code(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "Bridge 登录需要短信验证码；挑战内容不写日志");
    bridge_event(&payload, &client, &host, "login-smsCodeVerify").await;
}

pub async fn login_error(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::ERROR, "Bridge 登录失败；详情已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "login-error").await;
}

pub async fn login_slider(payload: Payload, client: Client, host: Arc<BotHost>) {
    event!(Level::WARN, "Bridge 登录需要滑块验证；挑战地址不写日志");
    bridge_event(&payload, &client, &host, "login-slider").await;
}

/// 兼容 Milky adapter 的额外 `login` 推送，并标记 QQ 已登录。
pub async fn bridge_login(_payload: Payload, _client: Client, host: Arc<BotHost>) {
    host.context.update_ica(|status| status.qq_login = true);
    event!(Level::INFO, "Milky bridge 已登录");
}

/// 处理好友申请、加群申请和群邀请，沿用同一个 Python hook。
///
/// add: 2.0.1
pub async fn join_request(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        match serde_json::from_value::<JoinRequestRoom>(value.clone()) {
            Ok(join_room) => {
                event!(Level::INFO, request_type = %join_room.request_type, sub_type = %join_room.sub_type, "收到好友或群申请");
                host.dispatch_ica_join_request(join_room, &client).await;
            }
            Err(error) => warn_invalid(&host, "sendAddRequest", Some(&error)),
        }
    }
}

/// 升级进度仅记录计数，不转储可能包含敏感内容的服务端文本。
pub async fn database_upgrade_progress(payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(value) = first_arg(&payload) {
        info!(active = ?value.get("active").and_then(JsonValue::as_bool),
            step = ?value.get("step").and_then(JsonValue::as_u64),
            total = ?value.get("total").and_then(JsonValue::as_u64), "Bridge 数据库升级进度");
    }
    bridge_event(&payload, &client, &host, "dbUpgradeProgress").await;
}

/// 必须在 Socket.IO 回调外等待真实 ACK，否则读循环无法消费回包。
async fn fetch_messages(client: &Client, room: RoomId, host: &BotHost, generation: u64) {
    if !host.is_accepting()
        || host.ica_connection().with_open_generation(generation, || ()).is_none()
    {
        return;
    }
    match bridge::fetch_messages(client, room, json!({}), Duration::from_secs(10)).await {
        Ok(values) => {
            if let Some(batch) = host.ica_connection().with_open_generation(generation, || {
                host.context
                    .update_ica(|status| event_state::store_messages(status, room, &values))
            }) {
                info!(
                    accepted = batch.accepted,
                    rejected = batch.rejected,
                    cached = batch.cached,
                    "当前房间消息查询已消费至有界缓存"
                );
            }
        }
        Err(_) => event!(Level::WARN, "当前房间消息查询失败或超时；缓存保持不变"),
    }
}

/// 专用处理器拥有状态变更/hook 的分发责任；on_any 不重复投递。
fn forwarded_event_name(event: &Event) -> Option<&str> {
    const HANDLED: &[&str] = &[
        "authSucceed",
        "authFailed",
        "authRequired",
        "requireAuth",
        "onlineData",
        "addMessage",
        "deleteMessage",
        "setAllRooms",
        "setMessages",
        "sendAddRequest",
        "dbUpgradeProgress",
        "messageSuccess",
        "messageError",
        "setOnline",
        "setOffline",
        "setShutUp",
        "renewMessage",
        "renewMessageURL",
        "requestSetup",
        "updateRoom",
        "notifyError",
        "fatal",
        "login-verify",
        "login-qrcodeLogin",
        "login-smsCodeVerify",
        "login-error",
        "login-slider",
        "login",
        "setAllChatGroups",
        "syncRead",
        "message",
    ];
    const GUI_ONLY: &[&str] = &["notify", "notifyMessage", "addMessageText", "closeLoading"];
    match event {
        Event::Custom(name)
            if !HANDLED.contains(&name.as_str()) && !GUI_ONLY.contains(&name.as_str()) =>
        {
            Some(name)
        }
        _ => None,
    }
}

/// 未有专用处理器的非 GUI 自定义事件保留 wire 参数，交给插件而非仅日志。
pub async fn any_event(event: Event, payload: Payload, client: Client, host: Arc<BotHost>) {
    if let Some(name) = forwarded_event_name(&event) {
        bridge_event(&payload, &client, &host, name).await;
    }
}

/// 处理认证阶段的通用 `message`、`authSucceed` 和 `authFailed` 回调。
pub async fn connect_callback(payload: Payload, _client: Client) {
    let span = span!(Level::INFO, "ica connect_callback");
    let _enter = span.enter();
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        match value.as_str() {
            Some("authSucceed") => {
                event!(Level::INFO, "{}", "已经登录到 icalingua!".green())
            }
            Some("authFailed") => {
                event!(Level::ERROR, "{}", "登录到 icalingua 失败!".red());
            }
            Some("authRequired") => {
                event!(Level::INFO, "{}", "需要登录到 icalingua!".yellow())
            }
            Some(_) => {
                event!(Level::DEBUG, "收到 Bridge 通用业务消息，正文不写日志");
            }
            _ => (),
        }
    }
}
