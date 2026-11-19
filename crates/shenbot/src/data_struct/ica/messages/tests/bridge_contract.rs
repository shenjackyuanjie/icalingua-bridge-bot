//! Socket.IO + oicq 消息转换与发送合同的业务边界。

use serde_json::{Value as JsonValue, json};

use super::{Message, NewMessage, SendMessage, raw::RawSendMessage};

fn message(id: JsonValue) -> JsonValue {
    json!({"_id": id, "username": "测试发送者", "content": "测试正文", "files": []})
}

#[test]
fn numeric_system_id_reaches_the_message_model_before_hook_dispatch() {
    let input = json!({
        "roomId": 10001,
        "message": {"_id": 1710000000, "content": "测试戳一戳", "system": true}
    });
    let event: NewMessage = serde_json::from_value(input).unwrap();
    assert_eq!(event.msg.msg_id, "1710000000");
    assert!(event.msg.system);
    assert_eq!(event.msg.sender_id, -1);
    assert!(event.msg.files.is_empty());
    assert_eq!(event.as_deleted().message_id, "1710000000");
    assert_eq!(event.msg.raw_data["_id"], 1710000000);
}

#[test]
fn malformed_core_fields_return_errors_and_do_not_poison_the_next_message() {
    let malformed = [
        JsonValue::Null,
        json!([]),
        json!({"_id": true}),
        json!({"_id": ""}),
        json!({"content": "没有 ID"}),
        json!({"_id": "bad", "username": {"wrong": "type"}}),
        json!({"_id": "bad", "content": []}),
        json!({"_id": "bad", "roomId": "不能把无效来源当作当前房间"}),
    ];
    for input in malformed {
        let parsed = std::panic::catch_unwind(|| serde_json::from_value::<Message>(input));
        let error = parsed.expect("畸形消息应返回错误而不是 panic").unwrap_err();
        assert!(!error.to_string().is_empty());
    }
    let next: Message = serde_json::from_value(message(json!("next"))).unwrap();
    assert_eq!(next.msg_id, "next");
    assert_eq!(next.content, "测试正文");
}

#[test]
fn history_file_without_url_remains_queryable_in_an_incoming_reply() {
    let input = json!({
        "_id": "live-reply", "username": "测试", "content": "引用旧文件",
        "replyMessage": {
            "_id": "history-file", "senderId": 10002, "content": "旧文件",
            "files": [{"type": "application/octet-stream", "fid": "synthetic-file", "size": 5368709120_u64}]
        }
    });
    let parsed = Message::new_from_json(&input).unwrap();
    assert_eq!(parsed.content, "引用旧文件");
    let reply = parsed.reply.unwrap().to_bridge_value();
    assert_eq!(reply["_id"], "history-file");
    assert_eq!(reply["senderId"], 10002);
    assert_eq!(reply["files"][0]["url"], "");
    assert_eq!(reply["files"][0]["fid"], "synthetic-file");
    assert_eq!(reply["files"][0]["size"], 5368709120_u64);
}

#[test]
fn malformed_attachment_is_isolated_and_valid_files_and_body_are_retained() {
    let input = json!({
        "_id": "mixed-media", "content": "正文仍能交给插件",
        "files": [false, {"type": "file", "size": -1},
            {"type": "image/png", "url": "https://example.invalid/image"},
            {"type": "application/octet-stream", "fid": "unresolved"}],
        "file": {"type": "audio/ogg", "url": "https://example.invalid/audio"}
    });
    let parsed = Message::new_from_json(&input).unwrap();
    assert_eq!(parsed.content, "正文仍能交给插件");
    assert_eq!(parsed.files.len(), 3);
    assert_eq!(parsed.files[0].file_type, "image/png");
    assert_eq!(parsed.files[1].fid.as_deref(), Some("unresolved"));
    assert_eq!(parsed.files[1].url, "");
    assert_eq!(parsed.files[2].file_type, "audio/ogg");
    assert_eq!(parsed.raw_data, input);

    for files in [json!(true), json!({"wrong": "type"}), JsonValue::Null] {
        let mut next = input.clone();
        next["files"] = files;
        let parsed = Message::new_from_json(&next).unwrap();
        assert_eq!(parsed.content, "正文仍能交给插件");
        assert_eq!(parsed.files.len(), 1, "坏列表不能连带丢弃合法旧单附件");
    }
}

#[test]
fn optional_metadata_and_bad_reply_do_not_turn_normal_text_into_a_dropped_message() {
    let input = json!({
        "_id": "optional", "content": "合法正文", "username": "测试",
        "senderId": {}, "role": [], "title": false, "time": i64::MAX,
        "at": "不是 all", "deleted": "false", "system": 1, "hide": {},
        "anonymousId": "invalid", "bubble_id": [], "subid": false,
        "button_rows": [[{"label": "缺 URL"}]], "replyMessage": {"_id": false}
    });
    let before = chrono::Utc::now();
    let parsed = Message::new_from_json(&input).unwrap();
    let after = chrono::Utc::now();
    assert_eq!(parsed.content, "合法正文");
    assert_eq!(parsed.sender_id, -1);
    assert_eq!(parsed.role, "unknown");
    assert!(parsed.title.is_empty());
    assert!(parsed.reply.is_none());
    assert_eq!(parsed.at, super::At::None, "未知字符串不能被当成 @全体");
    assert!(!parsed.deleted && !parsed.system && !parsed.hide);
    assert_eq!(parsed.anonymous_id, None);
    assert_eq!((parsed.bubble_id, parsed.subid), (1, 1));
    assert_eq!(parsed.button_rows, json!([]));
    assert!(before <= parsed.time && parsed.time <= after);
    assert_eq!(parsed.raw_data, input);
}

#[test]
fn legacy_reply_single_file_is_normalized_without_losing_reply_identity() {
    let input = json!({"_id": "reply", "content": "引用",
        "replyMessage": {"_id": 17, "content": "旧引用", "files": {
            "type": "application/octet-stream", "fid": "single-file"}}});
    let parsed = Message::new_from_json(&input).unwrap();
    let reply = parsed.reply.unwrap().to_bridge_value();
    assert_eq!(reply["_id"], "17");
    assert_eq!(reply["files"].as_array().unwrap().len(), 1);
    assert_eq!(reply["files"][0]["fid"], "single-file");
}

#[test]
fn nullable_legacy_fields_can_be_used_without_inventing_a_reply_sender() {
    let input = json!({
        "_id": "legacy", "username": null, "content": null,
        "files": null, "file": null, "replyMessage": null, "at": null,
        "title": null, "role": null, "time": null
    });
    let parsed = Message::new_from_json(&input).unwrap();
    let reply = parsed.as_reply().to_bridge_value();
    assert_eq!(reply["_id"], "legacy");
    assert_eq!(reply["content"], "");
    assert!(reply["files"].as_array().unwrap().is_empty());
    assert!(reply.get("senderId").is_none());
}

#[test]
fn legacy_file_is_merged_with_multimedia_without_truncating_size_or_metadata() {
    let large_file = json!({
        "type": "application/octet-stream", "url": "https://example.invalid/file",
        "fid": "large-file", "name": "测试大文件", "size": 5368709120_u64,
        "order": 3, "height": 720, "width": 1280, "isFace": false,
        "oicq_metadata": {"token_hint": "synthetic"}
    });
    let input = json!({
        "_id": "media", "content": "A😀B", "username": "测试发送者",
        "files": [{"type": "image/png", "url": "https://example.invalid/image", "order": 1}],
        "file": large_file
    });
    let parsed = Message::new_from_json(&input).unwrap();
    assert_eq!(parsed.files.len(), 2);
    assert_eq!(parsed.files[1].size, Some(5368709120));
    assert_eq!(parsed.files[1].order, Some(3));
    assert_eq!(parsed.files[1].width, Some(1280));
    assert_eq!(parsed.files[1].is_face, Some(false));
    let wire = parsed.to_bridge_value();
    assert_eq!(wire["files"][1]["size"], 5368709120_u64);
    assert_eq!(wire["files"][1]["oicq_metadata"]["token_hint"], "synthetic");
}

#[test]
fn mirrored_single_file_does_not_duplicate_a_multifile_attachment() {
    let file = json!({"type": "audio/ogg", "url": "https://example.invalid/audio", "fid": "voice"});
    let input = json!({"_id": "voice-message", "files": [file.clone()], "file": file});
    let parsed = Message::new_from_json(&input).unwrap();
    assert_eq!(parsed.files.len(), 1);
    assert_eq!(parsed.files[0].fid.as_deref(), Some("voice"));
}

#[test]
fn rich_message_and_protocol_chain_are_separate_and_survive_reply_construction() {
    let input = json!({
        "_id": 42, "senderId": 10002, "username": "测试发送者", "content": "# 标题",
        "roomId": -10003, "_roomName": "测试房间", "time": 1710000000000_i64,
        "markdown": true, "button_rows": [[{
            "label": "打开", "visited_label": "已打开", "url": "https://example.invalid/action",
            "oicq_extension": 1
        }]],
        "code": "<synthetic/>", "recallInfo": "{\"operator\":10002}",
        "mirai": {"kind": "synthetic"}, "anonymousflag": "synthetic-flag",
        "file": {"type": "image/png", "url": "https://example.invalid/image", "order": 0},
        "replyMessage": {"_id": 41, "senderId": 10004, "content": "原消息", "custom_reply": true},
        "custom_oicq": {"preserve": true}
    });
    let parsed = Message::new_from_json(&input).unwrap();
    assert!(parsed.raw_msg.is_null());
    assert_eq!(parsed.raw_data["custom_oicq"]["preserve"], true);
    assert!(parsed.markdown);
    assert_eq!(parsed.room_id, Some(-10003));
    assert_eq!(parsed.anonymous_flag.as_deref(), Some("synthetic-flag"));
    assert_eq!(parsed.button_rows[0][0]["oicq_extension"], 1);
    assert_eq!(parsed.reply.as_ref().unwrap().msg_id, "41");
    let outbound = SendMessage::new("回复".into(), -10003, Some(parsed.as_reply())).as_value();
    let reply = &outbound["replyMessage"];
    assert_eq!(reply["senderId"], 10002);
    assert_eq!(reply["files"].as_array().unwrap().len(), 1);
    assert_eq!(reply["replyMessage"]["senderId"], 10004);
    assert_eq!(reply["replyMessage"]["custom_reply"], true);
    assert_eq!(reply["button_rows"][0][0]["visited_label"], "已打开");
    assert_eq!(reply["custom_oicq"]["preserve"], true);
    assert_eq!(outbound["at"], json!([]));
}

#[test]
fn protocol_raw_chain_is_retained_only_as_a_valid_node_array() {
    let mut input = message(json!("raw-chain"));
    input["message"] = json!([{"type": "text", "text": "协议原文"}]);
    let parsed = Message::new_from_json(&input).unwrap();
    assert!(parsed.raw_msg.is_array());
    assert_eq!(parsed.raw_msg[0]["text"], "协议原文");
    input["message"] = json!({"_id": "not-a-chain"});
    let parsed = Message::new_from_json(&input).unwrap();
    assert!(parsed.raw_msg.is_null());
    assert_eq!(parsed.raw_data["message"]["_id"], "not-a-chain");
}

#[test]
fn message_patch_updates_full_media_and_preserves_unmentioned_rich_fields() {
    let mut input = message(json!("patch-target"));
    input["file"] = json!({"type": "audio/ogg", "url": "decoding", "fid": "voice"});
    input["markdown"] = json!(true);
    input["custom_oicq"] = json!({"keep": 1});
    let mut parsed = Message::new_from_json(&input).unwrap();
    parsed.apply_patch(&json!({
        "content": "语音已解码", "files": [{"type": "audio/ogg", "url": "https://example.invalid/voice", "fid": "voice"}],
        "deleted": true, "recallInfo": "{\"operator\":10002}",
        "replyMessage": {"_id": 1, "senderId": 10002, "content": "引用"}
    })).unwrap();
    assert_eq!(parsed.files.len(), 1);
    assert_eq!(parsed.files[0].url, "https://example.invalid/voice");
    assert!(parsed.file.is_none());
    assert!(parsed.deleted);
    assert!(parsed.markdown);
    assert_eq!(parsed.raw_data["custom_oicq"]["keep"], 1);
    assert_eq!(parsed.reply.as_ref().unwrap().msg_id, "1");
    assert!(parsed.recall_info.as_deref().unwrap().contains("operator"));
}

#[test]
fn invalid_message_patch_is_atomic_and_cannot_change_identity() {
    let mut parsed = Message::new_from_json(&message(json!("stable"))).unwrap();
    let before = parsed.to_bridge_value();
    for patch in [
        json!({"content": "不要部分写入", "files": [false]}),
        json!({"content": "不要部分写入", "files": true}),
        json!({"content": "不要部分写入", "time": i64::MAX}),
        json!({"content": "不要部分写入", "at": "不是 all"}),
        json!({"content": "不要部分写入", "replyMessage": {"_id": false}}),
        json!({"_id": "another", "deleted": true}),
        json!({"_id": null}),
        json!([{"deleted": true}]),
    ] {
        assert!(parsed.apply_patch(&patch).is_err());
        assert_eq!(parsed.to_bridge_value(), before);
    }
    parsed.apply_patch(&json!({"deleted": true})).unwrap();
    assert!(parsed.deleted);
}

#[test]
fn raw_python_entry_emits_string_content_and_at_array_for_flat_and_nested_nodes() {
    let input = r#"[
        {"type":"text","text":"测试"},
        {"type":"at","qq":10002},
        {"type":"xml","data":{"data":"<synthetic/>"}},
        {"type":"future_oicq_node","extension":true}
    ]"#;
    let wire = RawSendMessage::string_to_json(input, -10003).unwrap();
    assert_eq!(wire["messageType"], "raw");
    assert_eq!(wire["roomId"], -10003);
    assert_eq!(wire["at"], json!([]));
    let content = wire["content"].as_str().expect("Bridge 会 JSON.parse(content)");
    let nodes: JsonValue = serde_json::from_str(content).unwrap();
    assert_eq!(nodes.as_array().unwrap().len(), 4);
    assert_eq!(nodes[0]["text"], "测试");
    assert_eq!(nodes[1]["qq"], 10002);
    assert_eq!(nodes[2]["data"]["data"], "<synthetic/>");
    assert_eq!(nodes[3]["extension"], true);
}

#[test]
fn malformed_raw_inputs_never_become_a_sendable_wire_payload() {
    for input in [
        "not json",
        "null",
        "[]",
        "{}",
        r#"{"type":"text","text":"single"}"#,
        "[false]",
        r#"[{"text":"missing type"}]"#,
        r#"[{"type":false}]"#,
        r#"[{"type":" "}]"#,
    ] {
        assert!(RawSendMessage::string_to_json(input, -10003).is_err(), "{input}");
    }
}

#[test]
fn regular_send_keeps_reply_media_and_at_contract_after_message_model_changes() {
    let source = json!({
        "_id": "reply-source", "senderId": 10002, "content": "原消息",
        "file": {"type": "image/png", "url": "https://example.invalid/image"}
    });
    let source = Message::new_from_json(&source).unwrap();
    let mut outgoing = SendMessage::new("回复并附图".into(), -10003, Some(source.as_reply()));
    outgoing.set_img(b"synthetic-image", "image/png", true);
    outgoing.at = json!([{ "id": 10002, "text": "@测试发送者" }]);
    let wire = outgoing.as_value();
    assert_eq!(wire["replyMessage"]["senderId"], 10002);
    assert_eq!(wire["replyMessage"]["files"].as_array().unwrap().len(), 1);
    assert_eq!(wire["at"][0]["id"], 10002);
    assert_eq!(wire["media"][0]["type"], "image/png");
    assert!(wire["media"][0]["b64"].as_str().unwrap().starts_with("data:image/png;base64,"));
    assert!(wire.get("messageType").is_none());
    assert!(outgoing.has_base64_media());
}
