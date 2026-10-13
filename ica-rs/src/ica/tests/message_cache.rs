use serde_json::json;

use super::{DELETED_IDS, LoadedMessagesCache, message_size};
use crate::data_struct::ica::messages::Message;

fn message(id: &str, tick: i64) -> Message {
    Message::new_from_json(&json!({
        "_id": id, "content": id, "username": "测试", "time": 1_700_000_000_000_i64 + tick,
        "files": [],
    }))
    .unwrap()
}

fn ids(cache: &LoadedMessagesCache, room_id: i64) -> Vec<String> {
    cache
        .get(room_id, usize::MAX)
        .into_iter()
        .map(|message| message.msg_id)
        .collect()
}

#[test]
fn loaded_pages_merge_by_id_and_keep_rooms_and_latest_limit_isolated() {
    let mut cache = LoadedMessagesCache::with_limits(3, 3, usize::MAX);
    cache.store(-1, vec![message("b", 2), message("a", 1)], false);
    cache.store(-2, vec![message("b", 5)], false);
    let mut updated = message("b", 2);
    updated.content = "已更新".into();
    cache.store(-1, vec![updated, message("c", 3), message("old", 0)], false);

    assert_eq!(ids(&cache, -1), ["a", "b", "c"]);
    assert_eq!(ids(&cache, -2), ["b"]);
    assert_eq!(
        cache.get(-1, 2).iter().map(|item| item.msg_id.as_str()).collect::<Vec<_>>(),
        ["b", "c"]
    );
    assert_eq!(cache.get(-1, 2)[0].content, "已更新");
    assert!(cache.get(-1, 0).is_empty());
    assert_eq!(cache.counts().get(&-1), Some(&3));
}

#[test]
fn room_limit_evicts_least_recent_store_and_replace_only_affects_target_room() {
    let mut cache = LoadedMessagesCache::with_limits(2, 2, usize::MAX);
    cache.store(-1, vec![message("one", 1)], false);
    cache.store(-2, vec![message("two", 2)], false);
    cache.store(-1, vec![message("new-one", 3)], false);
    cache.store(-3, vec![message("three", 4)], false);
    assert!(cache.get(-2, 10).is_empty());
    assert_eq!(ids(&cache, -1), ["one", "new-one"]);

    cache.store(-1, vec![message("replacement", 5)], true);
    assert_eq!(ids(&cache, -1), ["replacement"]);
    assert_eq!(ids(&cache, -3), ["three"]);
}

#[test]
fn global_payload_budget_discards_old_loads_and_rejects_oversized_single_message() {
    let first = message("same-width-a", 1);
    let second = message("same-width-b", 2);
    let budget = message_size(&first).max(message_size(&second));
    let mut cache = LoadedMessagesCache::with_limits(10, 10, budget);
    cache.store(-1, vec![first], false);
    cache.store(-2, vec![second], false);
    assert!(cache.get(-1, 10).is_empty());
    assert_eq!(ids(&cache, -2), ["same-width-b"]);
    assert!(cache.payload_bytes <= budget);

    let mut oversized = message("huge", 3);
    oversized.content = "大".repeat(budget);
    assert_eq!(cache.store(-3, vec![oversized], false), 0);
    assert!(cache.get(-3, 10).is_empty());
    assert_eq!(ids(&cache, -2), ["same-width-b"]);
    assert_eq!(cache.store(0, vec![message("forward", 1)], false), 0);
}

#[test]
fn deletion_is_shared_across_wire_events_and_stale_history_cannot_revive_it() {
    let mut cache = LoadedMessagesCache::default();
    cache.store(-1, vec![message("deleted", 1)], false);
    assert!(cache.mark_deleted("deleted"));
    assert!(!cache.mark_deleted("deleted"));
    assert!(cache.get(-1, 1)[0].deleted);
    assert!(!cache.get(-1, 1)[0].reveal);
    cache.store(-1, vec![message("deleted", 1)], false);
    cache.store(-2, vec![message("deleted", 1)], false);
    assert!(cache.get(-1, 1)[0].deleted);
    assert!(cache.get(-2, 1)[0].deleted);

    cache.clear();
    assert!(cache.counts().is_empty());
    assert!(cache.mark_deleted("deleted"));
}

#[test]
fn deletion_dedup_has_a_bounded_window() {
    let mut cache = LoadedMessagesCache::default();
    for index in 0..=DELETED_IDS {
        assert!(cache.mark_deleted(&format!("message-{index}")));
    }
    assert_eq!(cache.deleted_ids.len(), DELETED_IDS);
    assert!(!cache.mark_deleted(&format!("message-{DELETED_IDS}")));
    assert!(cache.mark_deleted("message-0"));
}

#[test]
fn partial_updates_preserve_other_media_and_bad_patch_leaves_cached_message_intact() {
    let mut cache = LoadedMessagesCache::default();
    let value = json!({"_id":"media", "time":1_700_000_000_000_i64,
        "content":"旧", "files":[{"type":"image", "url":"https://example.invalid/old"}],
        "extension":{"source":"保留"}});
    cache.store(-1, vec![Message::new_from_json(&value).unwrap()], false);
    assert!(cache.patch(-1, "media", &json!({"content":"新"})).unwrap());
    let before = cache.get(-1, 1)[0].to_bridge_value();
    assert_eq!(before["files"][0]["url"], "https://example.invalid/old");
    assert_eq!(before["extension"]["source"], "保留");
    assert!(cache.patch(-1, "media", &json!({"files":true})).is_err());
    assert_eq!(cache.get(-1, 1)[0].to_bridge_value(), before);
    assert!(!cache.patch(-1, "not-loaded", &json!({"content":"不造占位"})).unwrap());
    assert_eq!(cache.counts().get(&-1), Some(&1));
}

#[test]
fn url_event_updates_primary_attachment_without_overwriting_unrelated_images() {
    let mut cache = LoadedMessagesCache::default();
    let value = json!({"_id":"album", "time":1_700_000_000_000_i64,
    "file":{"type":"image", "url":"https://example.invalid/primary"},
    "files":[
        {"type":"image", "url":"https://example.invalid/other"},
        {"type":"image", "url":"https://example.invalid/primary"}
    ]});
    cache.store(-1, vec![Message::new_from_json(&value).unwrap()], false);
    let updates = cache.renew_url("album", "https://example.invalid/refreshed");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, -1);
    let result = cache.get(-1, 1)[0].to_bridge_value();
    assert_eq!(result["file"]["url"], "https://example.invalid/refreshed");
    assert_eq!(result["files"][0]["url"], "https://example.invalid/other");
    assert_eq!(result["files"][1]["url"], "https://example.invalid/refreshed");
    assert!(cache.renew_url("not-loaded", "https://example.invalid/new").is_empty());
}
