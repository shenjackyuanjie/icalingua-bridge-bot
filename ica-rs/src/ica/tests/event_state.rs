use serde_json::{Value, json};

use super::{
    decode_add_message, delete_message, renew_message, renew_message_url, set_all_chat_groups,
    set_all_rooms, set_messages, sync_read, update_room,
};
use crate::data_struct::ica::messages::At;
use crate::status::ica::MainStatus;

fn message(id: &str, tick: i64) -> Value {
    json!({"_id":id,"content":id,"username":"测试","files":[],"time":1_700_000_000_000_i64+tick})
}

fn room(id: i64, unread: u64) -> Value {
    json!({"roomId":id,"roomName":"测试房间","index":0,"unreadCount":unread,
        "priority":3,"utime":1_700_000_000_000_i64,"lastMessage":{},"at":"all",
        "atMessageId":"定位消息","users":[{"_id":1}],"autoDownload":true,
        "downloadPath":"本地目录","extension":{"kept":true}})
}

#[test]
fn malformed_realtime_event_returns_error_instead_of_panicking() {
    for value in [
        json!(null),
        json!({"roomId":-1}),
        json!({"roomId":0,"message":message("id",1)}),
        json!({"roomId":-1,"message":{"_id":[],"content":"x"}}),
        json!({"roomId":-1,"message":{"_id":"id","files":true}}),
    ] {
        assert!(decode_add_message(&value).is_err());
    }
    let numeric_system = decode_add_message(&json!({"roomId":1,
        "message":{"_id":1_700_000_000_000_i64,"system":true,"content":"戳一戳"}}))
    .unwrap();
    assert_eq!(numeric_system.msg.msg_id, "1700000000000");
    assert!(numeric_system.msg.system);
}

#[test]
fn history_batches_skip_bad_entries_merge_loaded_pages_and_do_not_create_realtime_events() {
    let mut status = MainStatus::default();
    set_messages(&mut status, &json!({"roomId":-1,"messages":[message("live",3)]})).unwrap();
    let batch = set_messages(
        &mut status,
        &json!({"roomId":-1,"messages":[
        message("old",1), {"_id":true}, message("middle",2), message("live",3)]}),
    )
    .unwrap();
    assert_eq!(batch.accepted, 3);
    assert_eq!(batch.rejected, 1);
    assert_eq!(batch.cached, 3);
    let loaded = status.get_loaded_messages(-1, 10);
    assert_eq!(
        loaded.iter().map(|message| message.msg_id.as_str()).collect::<Vec<_>>(),
        ["old", "middle", "live"]
    );
    assert!(status.get_loaded_messages(-2, 10).is_empty());
    assert!(set_messages(&mut status, &json!({"roomId":-1,"messages":false})).is_err());
    assert_eq!(status.loaded_messages_count_for(-1), 3);
}

#[test]
fn qq_recall_patch_and_direct_delete_share_one_delete_hook_decision() {
    let mut status = MainStatus::default();
    set_messages(&mut status, &json!({"roomId":-1,"messages":[message("recall",1)]})).unwrap();
    let recall = json!({"roomId":-1,"messageId":"recall",
        "message":{"deleted":true,"reveal":false,"recallInfo":"脱敏撤回说明"}});
    let first = renew_message(&mut status, &recall).unwrap();
    assert!(first.first_deletion);
    assert_eq!(first.patch["recallInfo"], "脱敏撤回说明");
    assert!(!renew_message(&mut status, &recall).unwrap().first_deletion);
    assert!(delete_message(&mut status, &json!("recall")).unwrap().is_none());
    let cached = status.get_loaded_messages(-1, 10);
    assert!(cached[0].deleted);
    assert_eq!(cached[0].to_bridge_value()["recallInfo"], "脱敏撤回说明");

    assert_eq!(delete_message(&mut status, &json!(17)).unwrap(), Some("17".into()));
    let direct_first = json!({"roomId":-1,"messageId":17,"message":{"deleted":true}});
    assert!(!renew_message(&mut status, &direct_first).unwrap().first_deletion);
}

#[test]
fn normal_renew_preserves_full_loaded_message_and_unloaded_patch_never_fabricates_history() {
    let mut status = MainStatus::default();
    let mut original = message("media", 1);
    original["files"] = json!([{"type":"image","url":"https://example.invalid/one"}]);
    original["replyMessage"] = message("reply", 0);
    set_messages(&mut status, &json!({"roomId":-1,"messages":[original]})).unwrap();
    let update = renew_message(
        &mut status,
        &json!({"roomId":-1,"messageId":"media",
        "message":{"content":"已完成解码","extension":{"source":"更新"}}}),
    )
    .unwrap();
    assert!(!update.first_deletion);
    let before = status.get_loaded_messages(-1, 1)[0].to_bridge_value();
    assert_eq!(before["content"], "已完成解码");
    assert_eq!(before["files"][0]["url"], "https://example.invalid/one");
    assert_eq!(before["replyMessage"]["_id"], "reply");
    assert_eq!(before["extension"]["source"], "更新");
    assert!(
        renew_message(
            &mut status,
            &json!({"roomId":-1,"messageId":"media","message":{"files":true}})
        )
        .is_err()
    );
    assert_eq!(status.get_loaded_messages(-1, 1)[0].to_bridge_value(), before);

    let unseen = renew_message(
        &mut status,
        &json!({"roomId":-2,"messageId":"unseen","message":{"content":"后到补丁"}}),
    )
    .unwrap();
    assert_eq!(unseen.room_id, -2);
    assert!(!unseen.first_deletion);
    assert!(status.get_loaded_messages(-2, 10).is_empty());
}

#[test]
fn url_updates_infer_only_loaded_rooms_and_keep_wire_event_available_without_cache() {
    let mut status = MainStatus::default();
    let mut media = message("url", 1);
    media["files"] = json!([{"type":"image","url":"https://example.invalid/old"}]);
    set_messages(&mut status, &json!({"roomId":-1,"messages":[media]})).unwrap();
    let updates = renew_message_url(
        &mut status,
        &json!({"messageId":"url","URL":"https://example.invalid/new"}),
    )
    .unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, -1);
    assert_eq!(updates[0].1, "url");
    assert_eq!(updates[0].2["files"][0]["url"], "https://example.invalid/new");
    assert!(
        renew_message_url(
            &mut status,
            &json!({"messageId":"unseen","URL":"https://example.invalid/new"})
        )
        .unwrap()
        .is_empty()
    );
    assert!(renew_message_url(&mut status, &json!({"messageId":"url","URL":false})).is_err());
}

#[test]
fn room_snapshots_keep_unknown_business_fields_and_bad_snapshot_is_atomic() {
    let mut status = MainStatus::default();
    set_all_rooms(&mut status, &json!([room(-1, 5), room(-2, 7)])).unwrap();
    set_messages(&mut status, &json!({"roomId":-1,"messages":[message("kept",1)]})).unwrap();
    update_room(&mut status, &json!({"roomId":-1,"unreadCount":3})).unwrap();
    assert_eq!(status.rooms[0].unread_count, 3);
    assert_eq!(status.room_data[&-1]["users"], json!([{"_id":1}]));
    assert_eq!(status.room_data[&-1]["downloadPath"], "本地目录");
    assert_eq!(status.room_data[&-1]["extension"]["kept"], true);
    let mut invalid_room = room(-2, 7);
    invalid_room["priority"] = json!(9);
    assert!(set_all_rooms(&mut status, &json!([room(-1, 0), invalid_room])).is_err());
    assert_eq!(status.rooms.len(), 2);
    assert_eq!(status.rooms[0].unread_count, 3);
    assert_eq!(status.loaded_messages_count_for(-1), 1);
    assert!(update_room(&mut status, &json!({"roomId":-1,"unreadCount":"错误"})).is_err());
    assert_eq!(status.rooms[0].unread_count, 3);
}

#[test]
fn sync_read_clears_unread_and_at_only_in_its_room() {
    let mut status = MainStatus::default();
    set_all_rooms(&mut status, &json!([room(-1, 5), room(-2, 7)])).unwrap();
    assert_eq!(sync_read(&mut status, &json!(-1)).unwrap(), -1);
    assert_eq!(status.rooms[0].unread_count, 0);
    assert_eq!(status.rooms[0].at, At::Bool(false));
    assert_eq!(status.room_data[&-1]["atMessageId"], Value::Null);
    assert_eq!(status.rooms[1].unread_count, 7);
    assert_eq!(status.room_data[&-2]["atMessageId"], "定位消息");
    assert!(sync_read(&mut status, &json!(0)).is_err());
}

#[test]
fn chat_group_snapshot_preserves_extensions_and_bad_update_retains_last_good_state() {
    let mut status = MainStatus::default();
    let groups = json!([{"name":"默认","index":0,"rooms":[-1,2],"includeAllPersonal":true,
        "extension":{"color":"blue"}}]);
    set_all_chat_groups(&mut status, &groups).unwrap();
    assert_eq!(status.chat_groups[0]["extension"]["color"], "blue");
    assert!(
        set_all_chat_groups(&mut status, &json!([{"name":"错误","index":0,"rooms":true}])).is_err()
    );
    assert_eq!(status.chat_groups, groups.as_array().unwrap().clone());
}
