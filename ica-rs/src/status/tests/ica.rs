use std::collections::HashMap;

use serde_json::json;

use crate::data_struct::ica::messages::Message;
use crate::ica::event_state;
use crate::status::ica::MainStatus;

fn message(id: &str, tick: i64) -> Message {
    Message::new_from_json(
        &json!({"_id":id,"content":id,"files":[],"time":1_700_000_000_000_i64+tick}),
    )
    .unwrap()
}

fn room(id: i64) -> serde_json::Value {
    json!({"roomId":id,"roomName":"测试","index":0,"unreadCount":0,"priority":3,
        "utime":1_700_000_000_000_i64,"lastMessage":{}})
}

#[test]
fn loaded_count_is_saturating_and_missing_room_is_zero() {
    let mut status = MainStatus {
        loaded_messages_count_by_room: HashMap::from([(1, u64::MAX), (2, 10)]),
        ..Default::default()
    };
    assert_eq!(status.loaded_messages_count(), u64::MAX);
    assert_eq!(status.loaded_messages_count_for(2), 10);
    assert_eq!(status.loaded_messages_count_for(3), 0);
    status.clear_loaded_message_counts();
    assert_eq!(status.loaded_messages_count(), 0);
}

#[test]
fn public_query_store_counts_actual_unique_bounded_messages() {
    let mut status = MainStatus::default();
    let messages = (0..205).map(|tick| message(&format!("id-{tick}"), tick)).collect();
    status.store_loaded_messages(-1, messages, false);
    status.store_loaded_messages(-1, vec![message("id-204", 204)], false);
    status.store_loaded_messages(-2, vec![message("other", 1)], false);
    assert_eq!(status.loaded_messages_count_for(-1), 200);
    assert_eq!(status.loaded_messages_count(), 201);
    let latest = status.get_loaded_messages(-1, 2);
    assert_eq!(
        latest.iter().map(|item| item.msg_id.as_str()).collect::<Vec<_>>(),
        ["id-203", "id-204"]
    );
    status.store_loaded_messages(-1, vec![message("replacement", 206)], true);
    assert_eq!(status.loaded_messages_count_for(-1), 1);
    assert_eq!(status.loaded_messages_count_for(-2), 1);
}

#[test]
fn authoritative_room_removal_releases_loaded_cache_and_matching_counts() {
    let mut status = MainStatus::default();
    event_state::set_all_rooms(&mut status, &json!([room(-1), room(-2)])).unwrap();
    status.store_loaded_messages(-1, vec![message("one", 1)], false);
    status.store_loaded_messages(-2, vec![message("two", 2)], false);
    event_state::set_all_rooms(&mut status, &json!([room(-1)])).unwrap();
    assert_eq!(status.loaded_messages_count(), 1);
    assert!(status.get_loaded_messages(-2, 10).is_empty());
    assert!(!status.room_data.contains_key(&-2));
}

#[test]
fn reconnect_cleanup_discards_messages_and_resets_delete_hook_dedup() {
    let mut status = MainStatus::default();
    status.store_loaded_messages(-1, vec![message("deleted", 1)], false);
    assert!(status.mark_message_deleted("deleted"));
    assert!(!status.mark_message_deleted("deleted"));
    status.clear_loaded_message_counts();
    assert!(status.get_loaded_messages(-1, 10).is_empty());
    assert_eq!(status.loaded_messages_count(), 0);
    assert!(status.mark_message_deleted("deleted"));
}
