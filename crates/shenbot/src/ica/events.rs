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

use crate::data_struct::ica::RoomId;
use crate::data_struct::ica::all_rooms::JoinRequestRoom;
use crate::data_struct::ica::messages::MessageTrait;
use crate::data_struct::ica::online_data::OnlineData;
use crate::ica::client::{bridge, send_message};
use crate::ica::event_state;
use crate::py::host::PluginHost;
use crate::{VERSION, client_id, help_msg, py, version_str};

fn first_arg(payload: &Payload) -> Option<&JsonValue> {
    match payload {
        Payload::Text(values) => values.first(),
        _ => None,
    }
}

fn warn_invalid(host: &PluginHost, event_name: &'static str, error: Option<&serde_json::Error>) {
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
    host: &Arc<PluginHost>,
    event_name: &str,
) {
    py::call::ica_bridge_event_py(host, client, event_name, payload_args(payload)).await;
}

/// 获取在线数据
pub async fn get_online_data(payload: Payload, client: Client, state: (Arc<PluginHost>, u64)) {
    let (host, generation) = state;
    if let Some(value) = first_arg(&payload) {
        // 独立 oicq Bridge 只向 authed 房间发送 onlineData；无可靠账号不声明可发送。
        host.ica_connection.online_data(
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
pub async fn set_online(payload: Payload, client: Client, state: (Arc<PluginHost>, u64)) {
    let (host, generation) = state;
    host.ica_connection.set_online(generation, true);
    host.context.update_ica(|status| status.qq_login = true);
    event!(Level::INFO, "Icalingua 已上线");
    bridge_event(&payload, &client, &host, "setOnline").await;
}

/// 处理 `setOffline`，把本地 QQ 登录状态标记为离线并记录原因。
pub async fn set_offline(payload: Payload, client: Client, state: (Arc<PluginHost>, u64)) {
    let (host, generation) = state;
    host.ica_connection.set_online(generation, false);
    host.context.update_ica(|status| status.qq_login = false);
    event!(Level::WARN, "Icalingua 已离线，原因已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "setOffline").await;
}

/// 处理 `setShutUp`，记录当前会话的禁言状态变化。
pub async fn set_shut_up(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::INFO, shut_up = ?first_arg(&payload).and_then(JsonValue::as_bool), "当前会话禁言状态变化");
    bridge_event(&payload, &client, &host, "setShutUp").await;
}

/// 接收消息
pub(crate) async fn before_message_filters<T: Clone>(
    host: &Arc<PluginHost>,
    connection: &py::ica_connection::ConnectionState<T>,
    generation: u64,
    message: &crate::data_struct::ica::messages::NewMessage,
) -> bool {
    if let Some(snapshot) =
        py::ica_input::snapshot(connection, generation, message, chrono::Utc::now())
    {
        py::call::ica_message_candidate_py(host, &snapshot).await;
    }
    host.context.config.ica().filter_list.contains(&message.msg.sender_id)
}

pub async fn add_message(payload: Payload, client: Client, state: (Arc<PluginHost>, u64)) {
    let (host, generation) = state;
    if let Some(value) = first_arg(&payload) {
        let message = match event_state::decode_add_message(value) {
            Ok(message) => message,
            Err(error) => {
                warn_invalid(&host, "addMessage", Some(&error));
                return;
            }
        };
        // 传输候选在 sender/quote/self/普通机器人命令处理之前接纳；不重放历史。
        // 检测是否在过滤列表内
        if before_message_filters(&host, &host.ica_connection.current(), generation, &message).await
        {
            return;
        }
        println!("new_msg {}", message.to_string().cyan());
        host.context.update_ica(|status| {
            status.store_loaded_messages(message.room_id, vec![message.msg.clone()], false);
        });
        // 就在这里处理掉最基本的消息
        // 之后的处理交给插件
        let admin_list = &host.context.config.ica().admin_list;
        if !host.context.read_ica(|status| message.is_from_self(status.online_status.qqid))
            && !message.is_reply()
        {
            if message.content() == "/bot-rs" {
                let reply = message.reply_with(&version_str());
                send_message(&client, &reply, &host.context).await;
            } else if message.content() == "/bot-ls" {
                let reply = message.reply_with(&format!(
                    "shenbot-py v{}-{}\n{}",
                    VERSION,
                    client_id(),
                    if host.context.config.check_py() {
                        let storage = host.storage.lock().await;
                        storage.display_plugins(false)
                    } else {
                        "未启用 Python 插件".to_string()
                    }
                ));
                send_message(&client, &reply, &host.context).await;
            } else if message.content() == "/bot-permission" {
                let reply = message.reply_with(&format!(
                    "您的权限: {}",
                    if admin_list.contains(&message.sender_id()) {
                        "管理员"
                    } else {
                        "没啥"
                    }
                ));
                send_message(&client, &reply, &host.context).await;
            } else if message.content() == "/bot-help" {
                let reply = message.reply_with(&help_msg());
                send_message(&client, &reply, &host.context).await;
            }
            // else if message.content() == "/bot-uptime" {
            //     let duration = match start_up_time().elapsed() {
            //         Ok(d) => format!("{:?}", d),
            //         Err(e) => format!("出问题啦 {:?}", e),
            //     };
            //     let reply = message.reply_with(&format!(
            //         "shenbot 已运行: {}", duration
            //     ));
            //     send_message(&client, &reply, &host.context).await;
            // }
            else if admin_list.contains(&message.sender_id()) {
                // admin 区
                // 先判定是否为 admin
                let client_id = client_id();

                if message.content().starts_with(&format!("/bot-enable-{client_id}")) {
                    // 尝试获取后面的信息
                    if let Some((_, name)) = message.content().split_once(" ") {
                        let status = host.storage.lock().await.get_status(name);
                        let reply = match status {
                            None => message.reply_with("未找到插件"),
                            Some(true) => message.reply_with("无变化, 插件已经启用"),
                            Some(false) => match py::storage::set_plugin_status(&host, name, true)
                                .await
                            {
                                Ok(_) => message.reply_with("启用插件完成"),
                                Err(e) => message.reply_with(&format!("启用插件失败, 错误: \n{e}")),
                            },
                        };
                        send_message(&client, &reply, &host.context).await;
                    }
                } else if message.content().starts_with(&format!("/bot-disable-{client_id}")) {
                    if let Some((_, name)) = message.content().split_once(" ") {
                        let status = host.storage.lock().await.get_status(name);
                        let reply = match status {
                            None => message.reply_with("未找到插件"),
                            Some(false) => message.reply_with("无变化, 插件已经禁用"),
                            Some(true) => match py::storage::set_plugin_status(&host, name, false)
                                .await
                            {
                                Ok(_) => message.reply_with("禁用插件完成"),
                                Err(e) => message.reply_with(&format!("禁用插件失败, 错误: \n{e}")),
                            },
                        };
                        send_message(&client, &reply, &host.context).await;
                    }
                } else if message.content().starts_with(&format!("/bot-reload-{client_id}")) {
                    if let Some((_, name)) = message.content().split_once(" ") {
                        let status = host.storage.lock().await.get_status(name);
                        let reply = match status {
                            None => message.reply_with("未找到插件"),
                            Some(_) => match py::storage::reload_plugin(&host, name).await {
                                Ok(_) => message.reply_with("重载成功"),
                                Err(e) => message.reply_with(&format!("重载失败, 错误: \n{e}")),
                            },
                        };
                        send_message(&client, &reply, &host.context).await;
                    }
                } else if message.content() == "/bot-fetch" {
                    let reply = message.reply_with("正在更新当前群消息");
                    send_message(&client, &reply, &host.context).await;
                    fetch_messages(&client, message.room_id, &host).await;
                }
            }
        }
        // python 插件
        // 检测 sys
        if message.system() {
            py::call::ica_system_message_py(&host, &message, &client).await;
        } else {
            py::call::ica_new_message_py(&host, &message, &client).await;
        }
    } else {
        warn_invalid(&host, "addMessage", None);
    }
}

/// 历史推送只更新有界已加载缓存，不重放普通/系统新消息 hook。
pub async fn set_messages(payload: Payload, client: Client, host: Arc<PluginHost>) {
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
pub async fn delete_message(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::delete_message(status, value)) {
            Ok(Some(message_id)) => {
                py::call::ica_delete_message_py(&host, message_id, &client).await
            }
            Ok(None) => (),
            Err(error) => warn_invalid(&host, "deleteMessage", Some(&error)),
        }
    } else {
        warn_invalid(&host, "deleteMessage", None);
    }
}

/// 完整房间快照安全解析，同时保留 typed Room 以外的业务字段。
pub async fn update_all_room(payload: Payload, client: Client, host: Arc<PluginHost>) {
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
pub async fn update_room(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::update_room(status, value)) {
            Ok(()) => bridge_event(&payload, &client, &host, "updateRoom").await,
            Err(error) => warn_invalid(&host, "updateRoom", Some(&error)),
        }
    } else {
        warn_invalid(&host, "updateRoom", None);
    }
}

pub async fn update_chat_groups(payload: Payload, client: Client, host: Arc<PluginHost>) {
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

pub async fn sync_read(payload: Payload, client: Client, host: Arc<PluginHost>) {
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
pub async fn success_message(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Some(value) = first_arg(&payload) {
        println!("messageSuccess {}", value.to_string().green());
    }
    bridge_event(&payload, &client, &host, "messageSuccess").await;
}

pub async fn failed_message(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "收到 Bridge 操作失败提示，详情已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "messageError").await;
}

/// 消息补丁更新已加载缓存；QQ 撤回首次映射旧删除 hook，且不重放新消息。
pub async fn renew_message(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::renew_message(status, value)) {
            Ok(update) => {
                if update.first_deletion {
                    py::call::ica_delete_message_py(&host, update.message_id.clone(), &client)
                        .await;
                }
                py::call::ica_message_updated_py(
                    &host,
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
pub async fn renew_message_url(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Some(value) = first_arg(&payload) {
        match host.context.update_ica(|status| event_state::renew_message_url(status, value)) {
            Ok(updates) => {
                for (room_id, message_id, patch) in updates {
                    py::call::ica_message_updated_py(&host, &client, room_id, &message_id, &patch)
                        .await;
                }
                bridge_event(&payload, &client, &host, "renewMessageURL").await;
            }
            Err(error) => warn_invalid(&host, "renewMessageURL", Some(&error)),
        }
    } else {
        warn_invalid(&host, "renewMessageURL", None);
    }
}

pub async fn notify_error(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "收到 Bridge 错误通知，详情已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "notifyError").await;
}

pub async fn fatal_error(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::ERROR, "收到 Bridge 致命错误通知");
    bridge_event(&payload, &client, &host, "fatal").await;
}

pub async fn request_setup(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "Bridge 尚未完成账号配置；配置字段仅交给业务事件 hook");
    bridge_event(&payload, &client, &host, "requestSetup").await;
}

pub async fn login_verify(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "Bridge 登录需要设备验证；挑战内容不写日志");
    bridge_event(&payload, &client, &host, "login-verify").await;
}

pub async fn login_qrcode(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "Bridge 登录需要扫码；二维码内容不写日志");
    bridge_event(&payload, &client, &host, "login-qrcodeLogin").await;
}

pub async fn login_sms_code(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "Bridge 登录需要短信验证码；挑战内容不写日志");
    bridge_event(&payload, &client, &host, "login-smsCodeVerify").await;
}

pub async fn login_error(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::ERROR, "Bridge 登录失败；详情已交给业务事件 hook");
    bridge_event(&payload, &client, &host, "login-error").await;
}

pub async fn login_slider(payload: Payload, client: Client, host: Arc<PluginHost>) {
    event!(Level::WARN, "Bridge 登录需要滑块验证；挑战地址不写日志");
    bridge_event(&payload, &client, &host, "login-slider").await;
}

/// 兼容 Milky adapter 的额外 `login` 推送，并标记 QQ 已登录。
pub async fn bridge_login(_payload: Payload, _client: Client, host: Arc<PluginHost>) {
    host.context.update_ica(|status| status.qq_login = true);
    event!(Level::INFO, "Milky bridge 已登录");
}

/// 处理好友申请、加群申请和群邀请，沿用同一个 Python hook。
///
/// add: 2.0.1
pub async fn join_request(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        match serde_json::from_value::<JoinRequestRoom>(value.clone()) {
            Ok(join_room) => {
                event!(Level::INFO, request_type = %join_room.request_type, sub_type = %join_room.sub_type, "收到好友或群申请");
                py::call::ica_join_request_py(&host, join_room, &client).await;
            }
            Err(error) => warn_invalid(&host, "sendAddRequest", Some(&error)),
        }
    }
}

/// 升级进度仅记录计数，不转储可能包含敏感内容的服务端文本。
pub async fn database_upgrade_progress(payload: Payload, client: Client, host: Arc<PluginHost>) {
    if let Some(value) = first_arg(&payload) {
        info!(active = ?value.get("active").and_then(JsonValue::as_bool),
            step = ?value.get("step").and_then(JsonValue::as_u64),
            total = ?value.get("total").and_then(JsonValue::as_u64), "Bridge 数据库升级进度");
    }
    bridge_event(&payload, &client, &host, "dbUpgradeProgress").await;
}

/// /bot-fetch 使用与公开查询 API 相同的真实 ACK 等待，成功后更新缓存而不重放新消息。
pub async fn fetch_messages(client: &Client, room: RoomId, host: &Arc<PluginHost>) {
    match bridge::fetch_messages(client, room, json!({}), Duration::from_secs(10)).await {
        Ok(values) => {
            let batch = host
                .context
                .update_ica(|status| event_state::store_messages(status, room, &values));
            info!(
                accepted = batch.accepted,
                rejected = batch.rejected,
                cached = batch.cached,
                "当前房间消息查询已消费至有界缓存"
            );
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
pub async fn any_event(event: Event, payload: Payload, client: Client, host: Arc<PluginHost>) {
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
