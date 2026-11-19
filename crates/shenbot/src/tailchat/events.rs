//! Tailchat 消息、删除和更新事件处理器。

use std::sync::Arc;

use colored::Colorize;
use rust_socketio::asynchronous::Client;
use rust_socketio::{Event, Payload};
use tracing::{Level, event, info};

use crate::data_struct::tailchat::messages::ReceiveMessage;
use crate::data_struct::tailchat::status::{BotStatus, UpdateDMConverse};

use crate::commands::{self, Reply, Source};
use crate::tailchat::client::{emit_join_room, send_message};

/// 所有
pub async fn any_event(event: Event, payload: Payload, _client: Client, _status: Arc<BotStatus>) {
    let handled = [
        // 真正处理过的
        "notify:chat.message.add",
        "notify:chat.message.delete",
        "notify:chat.converse.updateDMConverse",
        // 也许以后会用到
        "notify:chat.message.update",
        "notify:chat.message.addReaction",
        "notify:chat.message.removeReaction",
        // 忽略的
        "notify:chat.inbox.append", // 被 @ 之类的事件
    ];
    match &event {
        Event::Custom(event_name) => {
            if handled.contains(&event_name.as_str()) {
                return;
            }
        }
        Event::Message => {
            match payload {
                Payload::Text(values) => {
                    if let Some(value) = values.first() {
                        if handled.contains(&value.as_str().unwrap()) {
                            return;
                        }
                        info!("收到消息 {}", value.to_string().yellow());
                    }
                }
                _ => {
                    return;
                }
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

/// 处理 `message` 事件。
pub async fn on_message(payload: Payload, client: Client, status: Arc<BotStatus>) {
    let host = &status.host;
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        let message: ReceiveMessage = match serde_json::from_value(value.clone()) {
            Ok(v) => v,
            Err(e) => {
                event!(Level::WARN, "tailchat_msg {}", value.to_string().red());
                event!(Level::WARN, "tailchat_msg {}", format!("{e:?}").red());
                return;
            }
        };
        event!(Level::INFO, "tailchat_msg {}", message.to_string().yellow());

        let command = commands::parse(
            &message.content,
            Source::Tailchat,
            host.context.client_id(),
            message.is_reply(),
            host.context.config.tailchat().admin_list.contains(&message.sender_id),
        );
        let worker = host.clone();
        commands::dispatch(host, command, move |reply| async move {
            if let Some(Reply::Text(text)) = reply {
                send_message(&client, &message.reply_with(&text), &worker.context).await;
            }
            worker.dispatch_tailchat_message(&message, &client).await;
        })
        .await;
    }
}
/// 处理 `msg_delete` 事件。
pub async fn on_msg_delete(payload: Payload, _client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        info!("删除消息 {}", value.to_string().red());
    }
}

/// 处理 `converse_update` 事件。
pub async fn on_converse_update(payload: Payload, client: Client) {
    if let Payload::Text(values) = payload
        && let Some(value) = values.first()
    {
        emit_join_room(&client).await;
        let update_info: UpdateDMConverse = match serde_json::from_value(value.clone()) {
            Ok(value) => value,
            Err(e) => {
                event!(Level::WARN, "tailchat updateDMConverse {}", value.to_string().red());
                event!(Level::WARN, "tailchat updateDMConverse {}", format!("{e:?}").red());
                return;
            }
        };
        info!("更新会话 {}", format!("{update_info:?}").cyan());
    }
}
