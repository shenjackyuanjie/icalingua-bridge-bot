use crate::MainStatus;
use crate::data_struct::ica::messages::{DeleteMessage, SendMessage};
use crate::data_struct::ica::{RoomId, RoomIdTrait, UserId};
use crate::error::{ClientResult, IcaError};

use colored::Colorize;
use ed25519_dalek::{Signature, Signer, SigningKey};
use rust_socketio::Payload;
use rust_socketio::asynchronous::Client;
use serde_json::{Value, json};
use tracing::{Level, event, span, warn};

/// "安全" 的 发送一条消息
pub async fn send_message(client: &Client, message: &SendMessage) -> bool {
    let value = message.as_value();
    match client.emit("sendMessage", value).await {
        Ok(_) => {
            event!(Level::INFO, "send_message {}", format!("{:#?}", message).cyan());
            true
        }
        Err(e) => {
            warn!("send_message faild:{}", format!("{:#?}", e).red());
            false
        }
    }
}

/// "安全" 的 删除一条消息
pub async fn delete_message(client: &Client, message: &DeleteMessage) -> bool {
    let value = message.as_value();
    match client.emit("deleteMessage", value).await {
        Ok(_) => {
            event!(Level::DEBUG, "delete_message {}", format!("{:#?}", message).yellow());
            true
        }
        Err(e) => {
            event!(Level::WARN, "delete_message faild:{}", format!("{:#?}", e).red());
            false
        }
    }
}

/// "安全" 的 获取历史消息
/// ```typescript
/// async fetchHistory(messageId: string, roomId: number, currentLoadedMessagesCount: number)
/// ```
// #[allow(dead_code)]
// pub async fn fetch_history(client: &Client, roomd_id: RoomId) -> bool { false }
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
        .unwrap_or(&Value::Null)
        .as_str()
        .unwrap_or("unknow");
    if server_protocol_version != crate::ica::ICA_PROTOCOL_VERSION {
        warn!(
            "服务器版本与兼容版本不一致\n服务器协议版本:{:?}\n兼容版本:{}",
            version.get("protocolVersion"),
            crate::ica::ICA_PROTOCOL_VERSION
        );
    }

    let auth_key = match &require_data.first() {
        Some(Value::String(auth_key)) => Ok(auth_key),
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

/// 签名回调
/// 失败的时候得 panic
pub async fn sign_callback(payload: Payload, client: Client) {
    inner_sign(payload, &client).await.expect("Faild to sign");
}

/// 向指定群发送签到信息
///
/// 只能是群啊, 不能是私聊
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
