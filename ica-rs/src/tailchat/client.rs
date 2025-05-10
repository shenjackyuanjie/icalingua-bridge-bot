use crate::data_struct::tailchat::messages::SendingMessage;
// use crate::data_struct::tailchat::{ConverseId, GroupId, MessageId, UserId};

use colored::Colorize;
use reqwest::multipart;
use rust_socketio::asynchronous::Client;
use serde_json::{Value, json};
use tracing::{Level, event, span};

pub async fn send_message(client: &Client, message: &SendingMessage) -> bool {
    let span = span!(Level::INFO, "tailchat send message");
    let _enter = span.enter();
    let mut value: Value = message.as_value();
    if message.contain_file() {
        // 处理文件
        let mut header = reqwest::header::HeaderMap::new();
        header.append(
            "X-Token",
            crate::MainStatus::global_tailchat_status().jwt_token.clone().parse().unwrap(),
        );
        let file_client = match reqwest::ClientBuilder::new().default_headers(header).build() {
            Ok(client) => client,
            Err(e) => {
                event!(Level::ERROR, "file_client build failed:{}", format!("{e:#?}").red());
                return false;
            }
        };
        // 感谢 https://stackoverflow.com/questions/65814450/how-to-post-a-file-using-reqwest
        let upload_url =
            format!("{}/upload", crate::MainStatus::global_config().tailchat().host.clone());
        let file_body =
            multipart::Part::stream(message.file.file_data()).file_name(message.file.file_name());
        let form_data = multipart::Form::new().part("file", file_body);

        event!(Level::INFO, "sending file message");
        let data = match file_client.post(&upload_url).multipart(form_data).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    match resp.text().await {
                        Ok(text) => match serde_json::from_str::<Value>(&text) {
                            Ok(json) => json,
                            Err(e) => {
                                event!(
                                    Level::ERROR,
                                    "file uploaded, but response parse failed:{}",
                                    format!("{e:#?}").red()
                                );
                                return false;
                            }
                        },
                        Err(e) => {
                            event!(
                                Level::ERROR,
                                "file uploaded, but failed to get response:{}",
                                format!("{e:#?}").red()
                            );
                            return false;
                        }
                    }
                } else {
                    event!(Level::ERROR, "file upload faild:{}", format!("{resp:#?}").red());
                    return false;
                }
            }
            Err(e) => {
                event!(
                    Level::ERROR,
                    "file upload failed while posting data:{}",
                    format!("{e:#?}").red()
                );
                return false;
            }
        };
        let content = format!(
            "{}{}",
            message.content,
            message.file.gen_markdown(data["url"].as_str().unwrap())
        );
        value["content"] = json!(content);
    }
    match client.emit("chat.message.sendMessage", value).await {
        Ok(_) => {
            event!(Level::DEBUG, "send message {}", format!("{message:#?}").cyan());
            true
        }
        Err(e) => {
            event!(Level::WARN, "send message failed:{}", format!("{e:#?}").red());
            false
        }
    }
}

pub async fn emit_join_room(client: &Client) -> bool {
    let span = span!(Level::INFO, "tailchat findAndJoinRoom");
    let _enter = span.enter();
    match client.emit("chat.converse.findAndJoinRoom", json!([])).await {
        Ok(_) => {
            event!(Level::INFO, "emiting join room");
            true
        }
        Err(e) => {
            event!(Level::WARN, "emit_join_room faild:{}", format!("{e:#?}").red());
            false
        }
    }
}
