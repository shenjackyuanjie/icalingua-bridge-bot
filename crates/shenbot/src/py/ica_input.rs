//! 实时候选消息的最小来源快照；不复制引用正文、昵称或历史，不猜测 self 人工来源。
use super::ica_connection::ConnectionState;
use crate::data_struct::ica::messages::{NewMessage, parse_bridge_timestamp};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

pub(crate) fn snapshot<T: Clone>(
    connection: &ConnectionState<T>,
    generation: u64,
    message: &NewMessage,
    received_at: DateTime<Utc>,
) -> Option<Value> {
    let account = connection.account.filter(|id| *id > 0)?;
    if !connection.open || connection.client.is_none() || connection.generation != generation {
        return None;
    }
    let msg = &message.msg;
    let quote_id = msg
        .raw_data
        .get("replyMessage")
        .and_then(|v| v.get("_id"))
        .and_then(Value::as_str);
    let command = msg.content.trim_start();
    if !["/answer", "/submit", "/decline"].iter().any(|prefix| {
        command
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    }) && quote_id.is_none()
    {
        return None;
    }
    // 本接口不是全量聊天旁路；超限在复制到 Python 之前拒绝，不产生日志正文。
    let oversized = msg.content.len() > 65536
        || msg.msg_id.len() > 90
        || quote_id.is_some_and(|id| id.len() > 90);
    let source_id = msg.raw_data.get("_id").and_then(Value::as_str).filter(|id| !id.is_empty());
    let source_time = msg
        .raw_data
        .get("time")
        .and_then(Value::as_i64)
        .and_then(parse_bridge_timestamp)
        .map(|time| time.to_rfc3339());
    // 普通聊天模型允许可选字段降级；交互候选不能把被降级丢掉的媒体/系统标记当纯文本。
    let flagged = ["system", "deleted", "markdown", "flash"].iter().any(|key| {
        msg.raw_data
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_bool() != Some(false))
    });
    let media = ["files", "button_rows"].iter().any(|key| {
        msg.raw_data
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|items| !items.is_empty()))
    }) || msg.raw_data.get("file").is_some_and(|v| !v.is_null());
    let unsupported = oversized
        || flagged
        || media
        || msg.system
        || msg.deleted
        || msg.anonymous_id.is_some()
        || msg.raw_data.get("anonymousId").is_some_and(|v| !v.is_null())
        || !msg.files.is_empty()
        || msg.markdown
        || msg.code != Value::Null
        || msg.raw_data.get("code").is_some_and(|code| !code.is_null())
        || msg
            .raw_data
            .get("replyMessage")
            .is_some_and(|reply| !reply.is_null() && quote_id.is_none())
        || msg.raw_data.get("anonymousflag").is_some_and(|flag| !flag.is_null())
        || msg.raw_data.get("message").is_some_and(|chain| !chain.is_null());
    Some(json!({
        "version": 1, "connection_generation": generation,
        "account": account, "room_id": message.room_id,
        "source_message_id": if oversized { None } else { source_id }, "sender_id": msg.sender_id,
        "is_self": msg.sender_id == account,
        "body": if oversized { "" } else { &msg.content }, "quote_id": if oversized { None } else { quote_id },
        "source_time": source_time, "received_at": received_at.to_rfc3339(),
        "unsupported": unsupported,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ica::event_state::decode_add_message;

    fn connection() -> ConnectionState<()> {
        ConnectionState {
            generation: 2,
            open: true,
            account: Some(10001),
            online: true,
            client: Some(()),
        }
    }
    fn message() -> NewMessage {
        decode_add_message(&json!({"roomId":-10002,"message":{"_id":"platform-source","senderId":10003,"content":"/answer ABC123 1  ","time":1792688400000_i64,"replyMessage":{"_id":"quote-source","content":"不要复制引用正文"}}})).unwrap()
    }
    #[test]
    fn snapshot_keeps_only_new_body_and_never_uses_receive_time_as_source_time() {
        let mut msg = message();
        let result = snapshot(&connection(), 2, &msg, Utc::now()).unwrap();
        assert_eq!(result["body"], "/answer ABC123 1  ");
        assert_eq!(result["quote_id"], "quote-source");
        assert!(!result.to_string().contains("不要复制引用正文"));
        assert_eq!(result["is_self"], false);
        msg.msg.raw_data.as_object_mut().unwrap().remove("time");
        let result = snapshot(&connection(), 2, &msg, Utc::now()).unwrap();
        assert!(result["source_time"].is_null());
        msg.msg.sender_id = 10001;
        assert_eq!(snapshot(&connection(), 2, &msg, Utc::now()).unwrap()["is_self"], true);
    }
    #[test]
    fn unrelated_history_old_connection_and_oversized_content_do_not_become_candidates() {
        let mut msg = message();
        assert!(snapshot(&connection(), 1, &msg, Utc::now()).is_none());
        msg.msg.content = "普通聊天".into();
        msg.msg.raw_data.as_object_mut().unwrap().remove("replyMessage");
        assert!(snapshot(&connection(), 2, &msg, Utc::now()).is_none());
        msg.msg.content = "/answering 不是命令".into();
        assert!(snapshot(&connection(), 2, &msg, Utc::now()).is_none());
        msg.msg.content = format!("/answer {}", "x".repeat(65536));
        let result = snapshot(&connection(), 2, &msg, Utc::now()).unwrap();
        assert_eq!(result["unsupported"], true);
        assert_eq!(result["body"], "");
    }
    #[test]
    fn non_text_or_anonymous_source_is_explicitly_unsupported_not_plain_text_answer() {
        let mut msg = message();
        msg.msg.anonymous_id = Some(9);
        assert_eq!(snapshot(&connection(), 2, &msg, Utc::now()).unwrap()["unsupported"], true);
        for (key, value) in [
            ("files", json!([{"invalid":"media"}])),
            ("file", json!({"invalid":"media"})),
            ("system", json!("invalid")),
            ("markdown", json!(true)),
            ("anonymousId", json!("invalid")),
        ] {
            let mut raw = message().msg.raw_data;
            raw[key] = value;
            let decoded = decode_add_message(&json!({"roomId":-10002,"message":raw})).unwrap();
            assert_eq!(
                snapshot(&connection(), 2, &decoded, Utc::now()).unwrap()["unsupported"],
                true,
                "{key}"
            );
        }
    }
}
