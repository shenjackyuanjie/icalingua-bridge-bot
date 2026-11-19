use super::*;
use std::future::ready;

#[tokio::test]
async fn resume_sends_only_missing_chunk_with_its_offset_and_hash_then_verifies_whole_file() {
    let mut data = vec![0; FILE_CHUNK_SIZE];
    data.extend_from_slice(b"abc");
    let mut calls = Vec::new();
    let mut count = 0;
    let hash = upload_file_with("test.bin", &data, |event, args, timeout| {
        calls.push((event, args.clone(), timeout));
        count += 1;
        let result = match count {
            1 => json!({"allSuccess": false, "uploaded": [0]}),
            2 => json!(true),
            3 => json!({"allSuccess": true, "uploaded": [0, FILE_CHUNK_SIZE]}),
            _ => panic!("不应新增上传请求"),
        };
        ready(Ok(result))
    })
    .await
    .unwrap();
    assert_eq!(
        calls.iter().map(|call| call.0).collect::<Vec<_>>(),
        ["requestUpload", "uploadFile", "requestUpload"]
    );
    assert_eq!(
        calls[1].1,
        vec![
            json!(hash),
            json!(FILE_CHUNK_SIZE),
            json!([97, 98, 99]),
            json!("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        ]
    );
    assert_eq!(calls[0].1, calls[2].1);
    assert_eq!(calls[0].1[0], json!("test.bin"));
    assert_eq!(calls[0].1[2], json!(FILE_CHUNK_SIZE + 3));
}

#[tokio::test]
async fn complete_cached_upload_needs_no_chunks_but_empty_new_file_still_needs_verification() {
    let mut cached_calls = 0;
    upload_file_with("cached.bin", b"cached", |event, _, _| {
        cached_calls += 1;
        assert_eq!(event, "requestUpload");
        ready(Ok(json!({"allSuccess": true, "uploaded": [0]})))
    })
    .await
    .unwrap();
    assert_eq!(cached_calls, 1);

    let mut empty_calls = 0;
    upload_file_with("empty.bin", b"", |event, _, _| {
        empty_calls += 1;
        assert_eq!(event, "requestUpload");
        ready(Ok(json!({"allSuccess": empty_calls == 2, "uploaded": []})))
    })
    .await
    .unwrap();
    assert_eq!(empty_calls, 2);
}

#[tokio::test]
async fn all_offsets_without_valid_whole_hash_are_overwritten_instead_of_reported_complete() {
    let mut events = Vec::new();
    let mut count = 0;
    upload_file_with("broken.bin", b"correct", |event, args, _| {
        events.push(event);
        count += 1;
        ready(Ok(match count {
            1 => json!({"allSuccess": false, "uploaded": [0]}),
            2 => {
                assert_eq!(args[1], json!(0));
                assert_eq!(args[2], json!(b"correct"));
                json!(true)
            }
            3 => json!({"allSuccess": true, "uploaded": [0]}),
            _ => panic!("不应新增请求"),
        }))
    })
    .await
    .unwrap();
    assert_eq!(events, ["requestUpload", "uploadFile", "requestUpload"]);
}

#[tokio::test(start_paused = true)]
async fn rejected_chunk_retries_only_idempotent_chunk_and_has_a_finite_attempt_budget() {
    let mut calls = Vec::new();
    let mut chunk_attempts = 0;
    let started = tokio::time::Instant::now();
    let error = upload_file_with("fail.bin", b"content", |event, _, _| {
        calls.push(event);
        ready(Ok(if event == "requestUpload" {
            json!({"allSuccess": false, "uploaded": []})
        } else {
            chunk_attempts += 1;
            json!(false)
        }))
    })
    .await
    .unwrap_err();
    assert!(error.contains("偏移 0"));
    assert_eq!(chunk_attempts, CHUNK_ATTEMPTS);
    assert_eq!(calls, ["requestUpload", "uploadFile", "uploadFile", "uploadFile"]);
    assert_eq!(started.elapsed(), Duration::from_millis(400));
}

#[tokio::test(start_paused = true)]
async fn lost_chunk_ack_can_be_retried_but_success_still_requires_final_checksum_ack() {
    let mut attempts = 0;
    let mut upload_requests = 0;
    let error = upload_file_with("pending.bin", b"content", |event, _, _| {
        let result = if event == "requestUpload" {
            upload_requests += 1;
            Ok(json!({"allSuccess": false, "uploaded": if upload_requests == 1 { vec![] } else { vec![0] }}))
        } else {
            attempts += 1;
            if attempts == 1 { Err("ACK 超时".to_string()) } else { Ok(json!(true)) }
        };
        ready(result)
    }).await.unwrap_err();
    assert_eq!(attempts, 2);
    assert_eq!(upload_requests, 2);
    assert!(error.contains("整文件 SHA-256"));
}

#[test]
fn resume_response_rejects_bad_offsets_and_false_completion() {
    for value in [
        json!({"allSuccess": false, "uploaded": [-1]}),
        json!({"allSuccess": false, "uploaded": [1]}),
        json!({"allSuccess": false, "uploaded": [FILE_CHUNK_SIZE]}),
        json!({"allSuccess": true, "uploaded": []}),
        json!({"uploaded": []}),
    ] {
        assert!(upload_status(value, 12).is_err());
    }
    assert!(upload_status(json!({"allSuccess": true, "uploaded": [0]}), 12).is_ok());
}

#[test]
fn ordinary_file_message_includes_required_at_and_never_routes_hash_into_media_path() {
    for mime in ["image/png", "audio/ogg"] {
        assert!(
            file_message(-123, "", None, vec![], mime, "hash", 12)
                .unwrap_err()
                .contains("媒体发送接口")
        );
    }
    let message =
        file_message(-123, "note", Some(json!({"_id": "reply"})), vec![], "", "hash", 12).unwrap();
    assert_eq!(message["at"], json!([]));
    assert_eq!(message["file"]["type"], json!("application/octet-stream"));
    assert_eq!(message["file"]["path"], json!("hash"));
    assert_eq!(message["roomId"], json!(-123));
    assert_eq!(message["replyMessage"]["_id"], json!("reply"));
    assert!(file_message(0, "", None, vec![], "text/plain", "hash", 0).is_err());
    assert_eq!(
        group_upload_arguments("hash", 123, "", "test.bin"),
        vec![json!("hash"), json!(123), json!("/"), json!("test.bin")]
    );
    assert_eq!(group_upload_arguments("hash", 123, "child", "test.bin")[2], json!("child"));
}

#[test]
fn actual_json_byte_chunk_exceeds_default_socket_limit_but_fits_four_mib_contract() {
    let bytes = vec![255_u8; FILE_CHUNK_SIZE];
    let packet = json!([
        "uploadFile",
        "0".repeat(64),
        9_007_199_254_740_991_u64,
        bytes,
        "f".repeat(64)
    ]);
    let encoded_size = packet.to_string().len() + 16; // Socket.IO ACK ID 及 Engine.IO 封装余量。
    assert!(encoded_size > 1_000_000);
    assert!(encoded_size < 2 * 1024 * 1024 + 256);
    assert!(encoded_size < 4 * 1024 * 1024);
}

#[test]
fn file_manager_enforces_its_separate_namespace_and_exact_mkdir_contract() {
    assert!(validate_file_manager("ls", &[json!("/"), json!(0)]).is_ok());
    assert!(validate_file_manager("ls", &[json!("/"), json!(-1)]).is_err());
    assert!(validate_file_manager("mkdir", &[json!("name")]).is_ok());
    assert!(validate_file_manager("mkdir", &[json!("name"), json!("parent")]).is_err());
    assert!(
        validate_file_manager(
            "uploadGroupFile",
            &[json!("hash"), json!(1), json!("/"), json!("a")]
        )
        .is_err()
    );
    assert!(validate_file_manager("download", &[json!("fid")]).is_ok());
}

#[test]
fn file_manager_auth_uses_event_positions_and_rejects_the_wrong_group() {
    assert!(
        file_authentication(Payload::Text(vec![json!(123), json!({"group_name": "fixture"})]), 123)
            .is_ok()
    );
    assert!(file_authentication(Payload::Text(vec![json!(124), json!({})]), 123).is_err());
    assert!(file_authentication(Payload::Text(vec![json!([123, {}])]), 123).is_err());
}

#[tokio::test]
async fn authentication_failure_never_runs_file_operation_and_duplicate_cannot_turn_it_into_success()
 {
    let (sender, receiver) = oneshot::channel();
    let responder = AuthResponder(Arc::new(Mutex::new(Some(sender))));
    responder.respond(Err("fileMgr 鉴权失败".into()));
    responder.respond(Ok(()));
    let error = after_authentication(receiver, Duration::from_secs(1), || async {
        panic!("鉴权失败时不应发目录操作");
        #[allow(unreachable_code)]
        Ok(Value::Null)
    })
    .await
    .unwrap_err();
    assert!(error.contains("鉴权失败"));
}

#[tokio::test(start_paused = true)]
async fn authentication_wait_has_a_real_local_deadline() {
    let (_sender, receiver) = oneshot::channel();
    let started = tokio::time::Instant::now();
    let error = after_authentication(receiver, Duration::from_secs(2), || async {
        panic!("鉴权超时时不应发操作");
        #[allow(unreachable_code)]
        Ok(Value::Null)
    })
    .await
    .unwrap_err();
    assert!(error.contains("鉴权超时"));
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test]
async fn successful_authentication_preserves_file_operation_error_instead_of_sent_success() {
    let (sender, receiver) = oneshot::channel();
    sender.send(Ok(())).unwrap();
    let error = after_authentication(receiver, Duration::from_secs(1), || async {
        bridge::checked_response("download", json!({"name": "expired", "url": "error"}))
    })
    .await
    .unwrap_err();
    assert!(error.contains("expired"));
}
