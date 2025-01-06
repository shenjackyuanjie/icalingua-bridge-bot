use colored::Colorize;
use rust_socketio::asynchronous::Client;
use rust_socketio::{Event, Payload};
use tracing::{event, info, span, warn, Level};

use crate::data_struct::ica::all_rooms::Room;
use crate::data_struct::ica::messages::{Message, MessageTrait, NewMessage};
use crate::data_struct::ica::online_data::OnlineData;
use crate::ica::client::send_message;
use crate::{client_id, help_msg, py, version_str, MainStatus, VERSION};

/// 获取在线数据
pub async fn get_online_data(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
            let online_data = OnlineData::new_from_json(value);
            event!(Level::DEBUG, "update_online_data {}", format!("{:?}", online_data).cyan());
            MainStatus::global_ica_status_mut().update_online_status(online_data);
        }
    }
}

/// 接收消息
pub async fn add_message(payload: Payload, client: Client) {
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
            let message: NewMessage = serde_json::from_value(value.clone()).unwrap();
            // 检测是否在过滤列表内
            if MainStatus::global_config().ica().filter_list.contains(&message.msg.sender_id) {
                return;
            }

            event!(Level::INFO, "new_msg {}", message.to_string().cyan());
            // 就在这里处理掉最基本的消息
            // 之后的处理交给插件
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
                            py::PyStatus::display()
                        } else {
                            "未启用 Python 插件".to_string()
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
                if MainStatus::global_config().ica().admin_list.contains(&message.sender_id()) {
                    // admin 区
                    // 先判定是否为 admin
                    let client_id = client_id();
                    if message.content().starts_with(&format!("/bot-enable-{}", client_id)) {
                        // 尝试获取后面的信息
                        if let Some((_, name)) = message.content().split_once(" ") {
                            match py::PyStatus::get().get_status(name) {
                                None => {
                                    let reply = message.reply_with("未找到插件");
                                    send_message(&client, &reply).await;
                                }
                                Some(true) => {
                                    let reply = message.reply_with("无变化, 插件已经启用");
                                    send_message(&client, &reply).await;
                                }
                                Some(false) => {
                                    py::PyStatus::get_mut().set_status(name, true);
                                    let reply = message.reply_with("启用插件完成");
                                    send_message(&client, &reply).await;
                                }
                            }
                        }
                    } else if message.content().starts_with(&format!("/bot-disable-{}", client_id))
                    {
                        if let Some((_, name)) = message.content().split_once(" ") {
                            match py::PyStatus::get().get_status(name) {
                                None => {
                                    let reply = message.reply_with("未找到插件");
                                    send_message(&client, &reply).await;
                                }
                                Some(false) => {
                                    let reply = message.reply_with("无变化, 插件已经禁用");
                                    send_message(&client, &reply).await;
                                }
                                Some(true) => {
                                    py::PyStatus::get_mut().set_status(name, false);
                                    let reply = message.reply_with("禁用插件完成");
                                    send_message(&client, &reply).await;
                                }
                            }
                        }
                    }
                }
            }
            // python 插件
            py::call::ica_new_message_py(&message, &client).await;
        }
    }
}

/// 理论上不会用到 (因为依赖一个客户端去请求)
/// 但反正实际上还是我去请求, 所以只是暂时
/// 加载一个房间的所有消息
pub async fn set_messages(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
            let messages: Vec<Message> = serde_json::from_value(value["messages"].clone()).unwrap();
            let room_id = value["roomId"].as_i64().unwrap();
            info!("set_messages {} len: {}", room_id.to_string().cyan(), messages.len());
        }
    }
}

/// 撤回消息
pub async fn delete_message(payload: Payload, client: Client) {
    if let Payload::Text(values) = payload {
        // 消息 id
        if let Some(value) = values.first() {
            if let Some(msg_id) = value.as_str() {
                event!(Level::INFO, "delete_message {}", msg_id.to_string().yellow());

                py::call::ica_delete_message_py(msg_id.to_string(), &client).await;
            }
        }
    }
}

pub async fn update_all_room(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
            if let Some(raw_rooms) = value.as_array() {
                let rooms: Vec<Room> = raw_rooms.iter().map(Room::new_from_json).collect();
                event!(Level::DEBUG, "update_all_room {}", rooms.len());
                MainStatus::global_ica_status_mut().update_rooms(rooms);
            }
        }
    }
}

pub async fn success_message(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
            info!("messageSuccess {}", value.to_string().green());
        }
    }
}

pub async fn failed_message(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
            warn!("messageFailed {}", value.to_string().red());
        }
    }
}

/// 所有
pub async fn any_event(event: Event, payload: Payload, _client: Client) {
    let handled = vec![
        // 真正处理过的
        "authSucceed",
        "authFailed",
        "authRequired",
        "requireAuth",
        "onlineData",
        "addMessage",
        "deleteMessage",
        "setAllRooms",
        "setMessages",
        // 也许以后会用到
        "messageSuccess",
        "messageFailed",
        "setAllChatGroups",
        "handleRequest", // 处理验证消息 (加入请求之类的)
        // 忽略的
        "notify",
        "syncRead",     // 同步已读
        "closeLoading", // 发送消息/加载新聊天 有一个 loading
        "renewMessage", // 我也不确定到底是啥事件
        "requestSetup", // 需要登录
        "updateRoom",   // 更新房间
    ];
    match &event {
        Event::Custom(event_name) => {
            if handled.contains(&event_name.as_str()) {
                return;
            }
        }
        Event::Message => {
            if let Payload::Text(values) = payload {
                if let Some(value) = values.first() {
                    if handled.contains(&value.as_str().unwrap()) {
                        return;
                    }
                    info!("收到消息 {}", value.to_string().yellow());
                }
            }
            return;
        }
        _ => (),
    }
    match payload {
        Payload::Binary(ref data) => {
            println!("event: {} |{:?}", event, data)
        }
        Payload::Text(ref data) => {
            print!("event: {}", event.as_str().purple());
            for value in data {
                println!("|{}", value);
            }
        }
        _ => (),
    }
}

pub async fn connect_callback(payload: Payload, _client: Client) {
    let span = span!(Level::INFO, "ica connect_callback");
    let _enter = span.enter();
    if let Payload::Text(values) = payload {
        if let Some(value) = values.first() {
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
}
