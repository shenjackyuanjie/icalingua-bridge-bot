use super::*;

fn wire_ack(value: Value) -> Payload {
    // handle_ack 将整包 ACK 实参数组再次装入 Payload::Text 的一个槽位。
    Payload::Text(vec![Value::Array(vec![value])])
}

#[test]
fn message_ack_preserves_empty_single_and_nested_arrays_as_one_result() {
    for messages in [
        json!([]),
        json!([{"_id": 17, "files": [{"fid": "voice", "order": 0}], "message": [{"type": "record"}]}]),
        json!([{"_id": "a"}, {"_id": "b", "button_rows": [[{"id": "next"}]]}]),
    ] {
        let result = decode_ack(wire_ack(messages.clone()));
        let actual = message_result("fetchMessages", result).unwrap();
        assert_eq!(Value::Array(actual), messages);
    }
}

#[test]
fn acknowledgment_does_not_recursively_flatten_business_arrays() {
    let expected = json!([[1, 2], [3]]);
    assert_eq!(decode_ack(wire_ack(expected.clone())), expected);
    assert_eq!(
        decode_ack(Payload::Text(vec![json!(["resource", {"page": 2}])])),
        json!(["resource", {"page": 2}])
    );
    assert_eq!(decode_ack(Payload::Text(vec![json!([])])), Value::Null);
}

#[test]
fn request_routing_rejects_pseudo_acks_wrong_namespaces_and_known_unavailable_events() {
    for event in ["sendMessage", "handleRequest", "fetchHistory", "deleteLoginDevice"] {
        assert!(validate_main_event(event, &[], true).unwrap_err().contains("不提供 ACK"));
        assert!(validate_main_event(event, &[], false).is_ok());
    }
    for event in ["ls", "download", "stat", "mkdir", "mv", "rm", "rename"] {
        assert!(validate_main_event(event, &[], false).unwrap_err().contains("fileMgr"));
    }
    for event in [
        "reportRead",
        "stopFetchingHistory",
        "preloadImages",
        "getPrivateFileUrl",
    ] {
        assert!(validate_main_event(event, &[], true).is_err());
        assert!(validate_main_event(event, &[], false).is_err());
    }
    assert!(validate_main_event("pluginExtension", &[json!([1, 2])], true).is_ok());
    assert!(validate_main_event("pluginExtension", &[Value::Null], false).is_ok());
}

#[test]
fn search_optional_values_keep_all_six_business_positions() {
    let args = search_arguments(0, "topic", 20, None, Some(123), None);
    assert_eq!(
        args,
        vec![
            json!(0),
            json!("topic"),
            json!(20),
            Value::Null,
            json!(123),
            Value::Null
        ]
    );
    assert!(validate_main_event("searchMessages", &args, true).is_ok());
    assert!(validate_main_event("searchMessages", &args[..5], true).is_err());
    assert!(
        validate_main_event(
            "fetchMessagesBySender",
            &[json!(-1), json!(2), json!(0), Value::Null],
            true
        )
        .is_ok()
    );
    assert!(validate_main_event("getForwardMsg", &[json!("resource")], true).is_err());
}

#[test]
fn cursor_contract_rejects_competing_directions_and_incomplete_cursor() {
    assert!(
        validate_page_options(
            json!({"before": {"time": 1, "id": "a"}, "after": {"time": 2, "id": "b"}})
        )
        .is_err()
    );
    assert!(validate_page_options(json!({"before": {"time": 1}})).is_err());
    assert!(validate_page_options(json!({"after": {"time": "old", "id": "a"}})).is_err());
    assert_eq!(validate_page_options(Value::Null).unwrap(), json!({}));
    let cursor = json!({"before": {"time": 12, "id": 7}});
    assert_eq!(validate_page_options(cursor.clone()).unwrap(), cursor);
}

#[test]
fn legacy_error_shapes_are_errors_but_legitimate_empty_and_false_values_remain_data() {
    for value in [
        json!({"error": "denied"}),
        json!({"ok": false}),
        json!({"name": "expired", "url": "error"}),
        json!("error"),
    ] {
        assert!(checked_response("download", value).is_err());
    }
    assert!(checked_response("uploadFile", json!(false)).is_err());
    assert_eq!(checked_response("pluginExtension", json!(false)).unwrap(), json!(false));
    assert_eq!(checked_response("fetchMessages", json!([])).unwrap(), json!([]));
    assert_eq!(checked_response("getRoom", Value::Null).unwrap(), Value::Null);
}

#[test]
fn forward_resource_error_placeholder_is_not_delivered_as_a_successful_message() {
    let error = json!([{"_id": 0, "senderId": 0, "username": "错误", "time": 0, "content": "resource expired", "files": []}]);
    assert!(forward_result(error).unwrap_err().contains("resource expired"));
    let ordinary =
        json!([{"_id": "group|9", "senderId": 42, "username": "错误", "time": 123, "files": []}]);
    assert!(forward_result(ordinary).is_ok());
    assert!(message_result("fetchMessages", json!([null])).is_err());
    assert!(message_result("fetchMessages", json!({"messages": []})).is_err());
}
