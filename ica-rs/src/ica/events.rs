//! Icalingua bridge 主动推送事件和 ACK 响应的处理函数。

use colored::Colorize;
use futures_util::future::BoxFuture;
use rust_socketio::asynchronous::Client;
use rust_socketio::{Event, Payload};
use serde_json::Value as JsonValue;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tracing::{Level, event, info, span, warn};

use crate::data_struct::ica::RoomId;
use crate::data_struct::ica::all_rooms::{JoinRequestRoom, Room};
use crate::data_struct::ica::messages::{Message, MessageTrait, NewMessage};
use crate::data_struct::ica::online_data::OnlineData;
use crate::ica::client::send_message;
use crate::py::PY_PLUGIN_STORAGE;
use crate::{MainStatus, VERSION, client_id, help_msg, py, version_str};

/// 获取在线数据
pub async fn get_online_data(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        let online_data = OnlineData::new_from_json(value);
        event!(Level::DEBUG, "update_online_data {}", format!("{online_data:?}").cyan());
        let status = MainStatus::global_ica_status_mut();
        status.qq_login = online_data.online;
        status.update_online_status(online_data);
    }
}

/// 处理 `setOnline`，把本地 QQ 登录状态标记为在线。
pub async fn set_online(_payload: Payload, _client: Client) {
    MainStatus::global_ica_status_mut().qq_login = true;
    event!(Level::INFO, "Icalingua 已上线");
}

/// 处理 `setOffline`，把本地 QQ 登录状态标记为离线并记录原因。
pub async fn set_offline(payload: Payload, _client: Client) {
    MainStatus::global_ica_status_mut().qq_login = false;
    event!(Level::WARN, "Icalingua 已离线: {payload:?}");
}

/// 处理 `setShutUp`，记录当前会话的禁言状态变化。
pub async fn set_shut_up(payload: Payload, _client: Client) {
    event!(Level::INFO, "setShutUp: {payload:?}");
}

/// 接收消息
pub async fn add_message(payload: Payload, client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        let message: NewMessage = serde_json::from_value(value.clone()).unwrap();
        // 检测是否在过滤列表内
        if MainStatus::global_config().ica().filter_list.contains(&message.msg.sender_id) {
            return;
        }

        println!("new_msg {}", message.to_string().cyan());
        // 就在这里处理掉最基本的消息
        // 之后的处理交给插件
        let admin_list = &MainStatus::global_config().ica().admin_list;
        if !message.is_from_self() && !message.is_reply() {
            if message.content() == "/bot-rs" {
                let reply = message.reply_with(&version_str());
                send_message(&client, &reply).await;
            } else if message.content() == "/bot-ls" {
                let reply = message.reply_with(&format!(
                    "shenbot-py v{}-{}\n{}",
                    VERSION,
                    client_id(),
                    if MainStatus::global_config().check_py() {
                        let storage = PY_PLUGIN_STORAGE.lock().await;
                        storage.display_plugins(false)
                    } else {
                        "未启用 Python 插件".to_string()
                    }
                ));
                send_message(&client, &reply).await;
            } else if message.content() == "/bot-permission" {
                let reply = message.reply_with(&format!(
                    "您的权限: {}",
                    if admin_list.contains(&message.sender_id()) {
                        "管理员"
                    } else {
                        "没啥"
                    }
                ));
                send_message(&client, &reply).await;
            } else if message.content() == "/bot-help" {
                let reply = message.reply_with(&help_msg());
                send_message(&client, &reply).await;
            }
            // else if message.content() == "/bot-uptime" {
            //     let duration = match start_up_time().elapsed() {
            //         Ok(d) => format!("{:?}", d),
            //         Err(e) => format!("出问题啦 {:?}", e),
            //     };
            //     let reply = message.reply_with(&format!(
            //         "shenbot 已运行: {}", duration
            //     ));
            //     send_message(&client, &reply).await;
            // }
            else if admin_list.contains(&message.sender_id()) {
                // admin 区
                // 先判定是否为 admin
                let client_id = client_id();
                let mut storage = PY_PLUGIN_STORAGE.lock().await;

                if message.content().starts_with(&format!("/bot-enable-{client_id}")) {
                    // 尝试获取后面的信息
                    if let Some((_, name)) = message.content().split_once(" ") {
                        let reply = match storage.get_status(name) {
                            None => message.reply_with("未找到插件"),
                            Some(true) => message.reply_with("无变化, 插件已经启用"),
                            Some(false) => match storage.set_status(name, true) {
                                Ok(_) => message.reply_with("启用插件完成"),
                                Err(e) => message.reply_with(&format!("启用插件失败, 错误: \n{e}")),
                            },
                        };
                        send_message(&client, &reply).await;
                    }
                } else if message.content().starts_with(&format!("/bot-disable-{client_id}")) {
                    if let Some((_, name)) = message.content().split_once(" ") {
                        let reply = match storage.get_status(name) {
                            None => message.reply_with("未找到插件"),
                            Some(false) => message.reply_with("无变化, 插件已经禁用"),
                            Some(true) => match storage.set_status(name, false) {
                                Ok(_) => message.reply_with("禁用插件完成"),
                                Err(e) => message.reply_with(&format!("禁用插件失败, 错误: \n{e}")),
                            },
                        };
                        send_message(&client, &reply).await;
                    }
                } else if message.content().starts_with(&format!("/bot-reload-{client_id}")) {
                    if let Some((_, name)) = message.content().split_once(" ") {
                        let reply = match storage.get_status(name) {
                            None => message.reply_with("未找到插件"),
                            Some(_) => {
                                let plugin = storage.storage.get_mut(name).unwrap();
                                match plugin.reload_self(Some(false)) {
                                    Ok(_) => message.reply_with("重载成功"),
                                    Err(e) => message.reply_with(&format!("重载失败, 错误: \n{e}")),
                                }
                            }
                        };
                        send_message(&client, &reply).await;
                    }
                } else if message.content() == "/bot-fetch" {
                    let reply = message.reply_with("正在更新当前群消息");
                    send_message(&client, &reply).await;
                    fetch_messages(&client, message.room_id).await;
                }
            }
        }
        // python 插件
        // 检测 sys
        if message.system() {
            py::call::ica_system_message_py(&message, &client).await;
        } else {
            py::call::ica_new_message_py(&message, &client).await;
        }
    }
}

/// 理论上不会用到 (因为依赖一个客户端去请求)
/// 但反正实际上还是我去请求, 所以只是暂时
/// 加载一个房间的所有消息
pub async fn set_messages(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        let messages: Vec<Message> = serde_json::from_value(value["messages"].clone()).unwrap();
        let room_id = value["roomId"].as_i64().unwrap();
        println!("set_messages {} len: {}", room_id.to_string().cyan(), messages.len());
    }
}

/// 撤回消息
pub async fn delete_message(payload: Payload, client: Client) {
    if let Payload::Text(values) = payload {
        // 消息 id
        if let Some(value) = values.first()
            && let Some(msg_id) = value.as_str()
        {
            event!(Level::INFO, "delete_message {}", msg_id.to_string().yellow());

            py::call::ica_delete_message_py(msg_id.to_string(), &client).await;
        }
    }
}

/// 处理 `setAllRooms`，使用 bridge 返回的完整房间列表刷新本地状态。
pub async fn update_all_room(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
        && let Some(raw_rooms) = value.as_array()
    {
        let rooms: Vec<Room> = raw_rooms.iter().map(Room::new_from_json).collect();
        event!(Level::DEBUG, "update_all_room {}", rooms.len());
        MainStatus::global_ica_status_mut().update_rooms(rooms);
    }
}

/// 处理 `updateRoom`，更新已有房间，或者插入尚未缓存的新房间。
pub async fn update_room(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        let room = Room::new_from_json(value);
        let rooms = &mut MainStatus::global_ica_status_mut().rooms;
        if let Some(current) = rooms.iter_mut().find(|current| current.room_id == room.room_id) {
            *current = room;
        } else {
            rooms.push(room);
        }
    }
}

/// 处理 `messageSuccess`，记录消息操作成功结果。
pub async fn success_message(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        println!("messageSuccess {}", value.to_string().green());
    }
}

/// 处理 `messageError`，记录消息及 bridge 操作失败原因。
pub async fn failed_message(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        warn!("messageError {}", value.to_string().red());
    }
}

/// 处理 `renewMessage`，记录 bridge 对消息内容的刷新结果。
pub async fn renew_message(payload: Payload, _client: Client) {
    event!(Level::DEBUG, "renewMessage: {payload:?}");
}

/// 处理 `renewMessageURL`，记录消息资源 URL 的刷新结果。
pub async fn renew_message_url(payload: Payload, _client: Client) {
    event!(Level::DEBUG, "renewMessageURL: {payload:?}");
}

/// 处理 `notifyError`，记录普通 bridge 错误通知。
pub async fn notify_error(payload: Payload, _client: Client) {
    event!(Level::WARN, "notifyError: {payload:?}");
}

/// 处理 `fatal`，记录要求客户端停止工作的 bridge 致命错误。
pub async fn fatal_error(payload: Payload, _client: Client) {
    event!(Level::ERROR, "bridge fatal: {payload:?}");
}

/// 处理 `requestSetup`，提示 bridge 尚未配置 QQ 账号。
pub async fn request_setup(payload: Payload, _client: Client) {
    event!(Level::WARN, "bridge 尚未完成账号配置: {payload:?}");
}

/// 处理 `login-verify`，记录设备验证登录请求。
pub async fn login_verify(payload: Payload, _client: Client) {
    event!(Level::WARN, "bridge 登录需要设备验证: {payload:?}");
}

/// 处理 `login-qrcodeLogin`，记录二维码登录请求。
pub async fn login_qrcode(payload: Payload, _client: Client) {
    event!(Level::WARN, "bridge 登录需要扫码: {payload:?}");
}

/// 处理 `login-smsCodeVerify`，记录短信验证码登录请求。
pub async fn login_sms_code(payload: Payload, _client: Client) {
    event!(Level::WARN, "bridge 登录需要短信验证码: {payload:?}");
}

/// 处理 `login-error`，记录 bridge 登录失败原因。
pub async fn login_error(payload: Payload, _client: Client) {
    event!(Level::ERROR, "bridge 登录失败: {payload:?}");
}

/// 处理 `login-slider`，记录滑块验证登录请求。
pub async fn login_slider(payload: Payload, _client: Client) {
    event!(Level::WARN, "bridge 登录需要滑块验证: {payload:?}");
}

/// 兼容 Milky adapter 的额外 `login` 推送，并标记 QQ 已登录。
pub async fn bridge_login(payload: Payload, _client: Client) {
    MainStatus::global_ica_status_mut().qq_login = true;
    event!(Level::INFO, "Milky bridge 已登录: {payload:?}");
}

/// 处理加群申请
///
/// add: 2.0.1
pub async fn join_request(payload: Payload, client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        match serde_json::from_value::<JoinRequestRoom>(value.clone()) {
            Ok(join_room) => {
                event!(Level::INFO, "{}", format!("收到加群申请 {join_room:?}").on_blue());
                py::call::ica_join_request_py(join_room, &client).await;
            }
            Err(e) => {
                event!(
                    Level::WARN,
                    "呼叫 shenjack! JoinRequestRoom 的 serde 没写好! {}\nraw: {:#?}",
                    e,
                    value
                )
            }
        }
    }
}

// pub async fn fetch_history(client: Client, room: RoomId) {
//     let request_body = json!(room);
// }

/// 展开 rust-socketio 在 ACK payload 外层包装的参数数组。
fn ack_payload_values(payload: &Payload) -> Vec<JsonValue> {
    match payload {
        Payload::Text(values) => {
            if let Some(JsonValue::Array(args)) = values.first()
                && values.len() == 1
            {
                return args.clone();
            }
            values.clone()
        }
        Payload::Binary(bytes) => vec![json!(bytes.to_vec())],
        _ => Vec::new(),
    }
}

/// 通过 `fetchMessages(roomId, offset, ack)` 拉取房间消息并解析 ACK。
pub async fn fetch_messages(client: &Client, room: RoomId) {
    let timeout = Duration::from_secs(10);
    let ack_received = Arc::new(AtomicBool::new(false));
    let ack_received_cb = ack_received.clone();

    match client
        .emit_with_ack(
            "fetchMessages",
            vec![json!(room), json!(0)],
            timeout,
            move |payload: Payload, _client: Client| -> BoxFuture<'static, ()> {
                let ack_received = ack_received_cb.clone();
                Box::pin(async move {
                    ack_received.store(true, Ordering::SeqCst);
                    let ack_values = ack_payload_values(&payload);
                    let messages =
                        ack_values.first().cloned().unwrap_or_else(|| JsonValue::Array(Vec::new()));

                    match serde_json::from_value::<Vec<Message>>(messages) {
                        Ok(messages) => {
                            event!(Level::INFO, "fetch_messages {room} len: {}", messages.len());
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "fetch_messages {room} ACK 格式错误: {e}; raw: {ack_values:#?}"
                            );
                        }
                    }
                })
            },
        )
        .await
    {
        Ok(_) => {
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                if !ack_received.load(Ordering::SeqCst) {
                    event!(Level::WARN, "fetch_messages {room} ACK 超时");
                }
            });
        }
        Err(e) => {
            event!(Level::WARN, "fetch_messages {}", e);
        }
    }
}

/// 记录没有专用处理器的 Socket.IO 事件，并过滤已经处理或明确忽略的事件。
pub async fn any_event(event: Event, payload: Payload, _client: Client) {
    let handled = vec![
        // 真正处理过的
        "authSucceed",    // bridge 身份认证成功
        "authFailed",     // bridge 身份认证失败
        "authRequired",   // 旧版 bridge 要求客户端认证
        "requireAuth",    // bridge 下发签名盐和协议版本
        "onlineData",     // QQ 在线状态、账号和 bridge 信息
        "addMessage",     // 收到一条新消息
        "deleteMessage",  // 一条消息被撤回或删除
        "setAllRooms",    // 下发完整房间列表
        "setMessages",    // 下发指定房间的消息列表
        "sendAddRequest", // bridge 推送好友或群申请
        // 也许以后会用到
        "messageSuccess",      // 消息或相关操作执行成功
        "messageError",        // 消息或相关操作执行失败
        "setOnline",           // QQ 账号进入在线状态
        "setOffline",          // QQ 账号进入离线状态
        "setShutUp",           // 当前群聊禁言状态发生变化
        "renewMessage",        // bridge 刷新消息内容
        "renewMessageURL",     // bridge 刷新消息资源 URL
        "requestSetup",        // bridge 尚未配置 QQ 账号
        "updateRoom",          // 单个房间信息发生变化
        "notifyError",         // bridge 普通错误通知
        "fatal",               // bridge 致命错误通知
        "login-verify",        // QQ 登录需要设备验证
        "login-qrcodeLogin",   // QQ 登录需要扫码
        "login-smsCodeVerify", // QQ 登录需要短信验证码
        "login-error",         // QQ 登录失败
        "login-slider",        // QQ 登录需要滑块验证
        "login",               // Milky adapter 的额外登录成功通知
        // 面向 GUI，用不到
        "setAllChatGroups", // GUI 聊天分组列表
        "notify",           // GUI 桌面通知
        "notifyMessage",    // GUI 应用内消息通知
        "addMessageText",   // GUI 向消息区域追加提示文本
        "closeLoading",     // GUI 关闭加载状态
        "syncRead",         // GUI 同步房间已读状态
    ];
    match &event {
        Event::Custom(event_name) => {
            if handled.contains(&event_name.as_str()) {
                return;
            }
        }
        Event::Message => {
            if let Payload::Text(values) = payload
                && let Some(value) = values.first()
            {
                if handled.contains(&value.as_str().unwrap()) {
                    return;
                }
                info!("收到消息 {}", value.to_string().yellow());
            }
            return;
        }
        _ => (),
    }
    match payload {
        Payload::Binary(ref data) => {
            println!("event: {event} |{data:?}")
        }
        Payload::Text(ref data) => {
            print!("event: {}", event.as_str().purple());
            for value in data {
                println!("|{value}");
            }
        }
        _ => (),
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
                panic!("登录失败")
            }
            Some("authRequired") => {
                event!(Level::INFO, "{}", "需要登录到 icalingua!".yellow())
            }
            Some(msg) => {
                event!(Level::INFO, "{}{}", "未知消息".yellow(), msg);
            }
            _ => (),
        }
    }
}
