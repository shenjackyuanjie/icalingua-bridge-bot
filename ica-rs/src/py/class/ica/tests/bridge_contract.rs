//! 验证 Python API 对 Bridge 消息归属、节点重建与参数的业务约束。

use super::*;

fn message(value: Value) -> Message { Message::new_from_json(&value).expect("有效业务夹具") }

#[test]
fn room_queries_reject_foreign_messages_before_caching() {
    let result = decode_query_messages(vec![json!({"_id":"history", "roomId":-2})], -1, true);
    assert!(result.is_err());
    let items =
        decode_query_messages(vec![json!({"_id":123, "content":"旧历史"})], -1, true).unwrap();
    assert_eq!(items[0].room_id, -1);
    assert_eq!(items[0].msg.msg_id, "123");
}

#[test]
fn cross_room_query_preserves_each_result_source() {
    let items = decode_query_messages(
        vec![json!({"_id":"a", "roomId":-1}), json!({"_id":"b", "roomId":2})],
        0,
        true,
    )
    .unwrap();
    assert_eq!(items.iter().map(|item| item.room_id).collect::<Vec<_>>(), vec![-1, 2]);
    assert!(decode_query_messages(vec![json!({"_id":"missing-source"})], 0, true).is_err());
    assert_eq!(
        decode_query_messages(vec![json!({"_id":"forward-resource"})], 0, false).unwrap()[0]
            .room_id,
        0
    );
}

#[test]
fn malformed_history_is_a_query_error_not_a_partial_cache_write() {
    let result = decode_query_messages(vec![json!({"_id":"ok"}), json!({"_id":[]})], -1, true);
    assert!(result.is_err());
}

#[test]
fn forwarding_prefers_protocol_chain_and_uses_seconds() {
    let chain = json!([{"type":"face", "data":{"id":14}}]);
    let msg = message(json!({
        "_id":"source", "senderId":123, "username":"昵称", "time":1700000000123i64,
        "content":"已渲染文本", "message":chain,
        "files":[{"type":"file", "url":"https://example.invalid/private", "name":"文档"}],
    }));
    let node = forward_node(&msg);
    assert_eq!(node["message"], chain);
    assert_eq!(node["time"], 1700000000i64);
    assert_eq!(node["user_id"], 123);
    assert_eq!(node["id"], "source");
}

#[test]
fn forwarding_preserves_images_and_marks_unsupported_files_without_forging_resources() {
    let msg = message(json!({
        "_id":"attachments", "senderId":123, "content":"正文",
        "files":[
            {"type":"image/png", "url":"https://example.invalid/image"},
            {"type":"file", "url":"https://example.invalid/private", "name":"文档.pdf"},
        ],
    }));
    let node = forward_node(&msg);
    let elements = node["message"].as_array().unwrap();
    assert!(
        elements.iter().any(|item| item["type"] == "image"
            && item["data"]["file"] == "https://example.invalid/image")
    );
    assert!(
        elements
            .iter()
            .any(|item| item["type"] == "text"
                && item["data"]["text"] == "[文档.pdf 不支持合并转发]")
    );
    assert!(!elements.iter().any(|item| item["type"] == "file"));
    assert!(!node.to_string().contains("https://example.invalid/private"));
}

#[test]
fn forwarding_restores_reply_and_embedded_card_without_duplicate_rendered_text() {
    let msg = message(json!({
        "_id":"card", "senderId":123, "content":"卡片摘要", "code":"<msg>卡片</msg>",
        "replyMessage":{"_id":"origin", "content":"原文", "username":"引用者", "files":[]},
    }));
    let node = forward_node(&msg);
    assert_eq!(node["message"][0]["type"], "reply");
    assert_eq!(node["message"][0]["data"]["id"], "origin");
    assert_eq!(node["message"][1]["type"], "xml");
    assert!(
        !node["message"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["data"]["text"] == "卡片摘要")
    );
}

#[test]
fn sending_mentions_and_forward_nodes_enforce_real_wire_shapes() {
    assert!(validate_mentions(&json!([{"id":123, "text":"昵称"}, {"id":"all"}])).is_ok());
    assert!(validate_mentions(&json!(true)).is_err());
    assert!(validate_mentions(&json!([{"id":0}])).is_err());
    assert!(validate_mentions(&json!([{"id":123, "text":false}])).is_err());
    let node = json!([{"user_id":123, "message":[{"type":"dice"}], "time":1700000000}]);
    assert!(validate_forward_nodes(&node, -1).is_ok());
    assert!(validate_forward_nodes(&node, 0).is_err());
    assert!(validate_forward_nodes(&json!([]), -1).is_err());
    assert!(validate_forward_nodes(&json!([{"user_id":123,"message":[]}]), -1).is_err());
}

#[test]
fn query_deadlines_reject_unbounded_and_non_finite_waits() {
    assert_eq!(query_timeout(0.25).unwrap(), Duration::from_millis(250));
    for seconds in [0.0, -1.0, 301.0, f64::INFINITY, f64::NAN] {
        assert!(query_timeout(seconds).is_err());
    }
}
