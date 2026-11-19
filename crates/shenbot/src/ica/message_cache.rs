//! 只保存已经加载的 Bridge 消息；历史查询不重放实时消息 hook。

#[cfg(test)]
#[path = "tests/message_cache.rs"]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{Value, json};

use crate::data_struct::ica::messages::Message;

const MESSAGES_PER_ROOM: usize = 200;
const LOADED_ROOMS: usize = 64;
const PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const DELETED_IDS: usize = 4096;
const DELETED_ID_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
struct CachedMessage {
    message: Message,
    payload_bytes: usize,
    sequence: u64,
}

#[derive(Debug, Clone, Default)]
struct LoadedRoom {
    messages: Vec<CachedMessage>,
    last_access: u64,
}

/// 按房间隔离、按消息 ID 去重的有界缓存。
///
/// 字节预算按完整规范化 JSON 的序列化长度估算；另有消息条数和房间数上限，
/// 不作为 Rust 堆占用的精确测量。撤回墓碑独立有界，避免旧历史回包复活已撤回消息。
#[derive(Debug, Clone)]
pub struct LoadedMessagesCache {
    rooms: HashMap<i64, LoadedRoom>,
    clock: u64,
    payload_bytes: usize,
    messages_per_room: usize,
    max_rooms: usize,
    max_payload_bytes: usize,
    deleted_ids: HashSet<String>,
    deleted_order: VecDeque<String>,
    deleted_id_bytes: usize,
}

impl Default for LoadedMessagesCache {
    fn default() -> Self { Self::with_limits(MESSAGES_PER_ROOM, LOADED_ROOMS, PAYLOAD_BYTES) }
}

impl LoadedMessagesCache {
    fn with_limits(messages_per_room: usize, max_rooms: usize, max_payload_bytes: usize) -> Self {
        Self {
            rooms: HashMap::new(),
            clock: 0,
            payload_bytes: 0,
            messages_per_room,
            max_rooms,
            max_payload_bytes,
            deleted_ids: HashSet::new(),
            deleted_order: VecDeque::new(),
            deleted_id_bytes: 0,
        }
    }

    /// 合并查询/推送得到的消息；replace 仅用于调用者明确要替换已加载集合时。
    pub fn store(&mut self, room_id: i64, messages: Vec<Message>, replace: bool) -> usize {
        if room_id == 0 || self.messages_per_room == 0 || self.max_rooms == 0 {
            return 0;
        }
        if replace {
            self.remove_room(room_id);
        }
        let mut accepted = 0;
        for mut message in messages {
            if self.deleted_ids.contains(&message.msg_id) {
                let _ = message.apply_patch(&json!({"deleted": true, "reveal": false}));
            }
            let payload_bytes = message_size(&message);
            if payload_bytes > self.max_payload_bytes {
                continue;
            }
            self.clock = self.clock.saturating_add(1);
            let room = self.rooms.entry(room_id).or_default();
            room.last_access = self.clock;
            let sequence = if let Some(index) =
                room.messages.iter().position(|cached| cached.message.msg_id == message.msg_id)
            {
                let previous = room.messages.remove(index);
                self.payload_bytes -= previous.payload_bytes;
                previous.sequence
            } else {
                self.clock
            };
            room.messages.push(CachedMessage {
                message,
                payload_bytes,
                sequence,
            });
            room.messages.sort_by_key(|cached| (cached.message.time, cached.sequence));
            self.payload_bytes += payload_bytes;
            while room.messages.len() > self.messages_per_room {
                self.payload_bytes -= room.messages.remove(0).payload_bytes;
            }
            accepted += 1;
            self.enforce_limits();
        }
        accepted
    }

    /// 返回最新 limit 条消息，保持时间升序；返回值为独立快照。
    pub fn get(&self, room_id: i64, limit: usize) -> Vec<Message> {
        self.rooms.get(&room_id).map_or_else(Vec::new, |room| {
            room.messages
                .iter()
                .skip(room.messages.len().saturating_sub(limit))
                .map(|cached| cached.message.clone())
                .collect()
        })
    }

    pub fn counts(&self) -> HashMap<i64, u64> {
        self.rooms
            .iter()
            .map(|(room_id, room)| (*room_id, room.messages.len() as u64))
            .collect()
    }

    pub fn retain_rooms(&mut self, room_ids: &HashSet<i64>) {
        let removed: Vec<_> = self
            .rooms
            .keys()
            .filter(|room_id| !room_ids.contains(room_id))
            .copied()
            .collect();
        for room_id in removed {
            self.remove_room(room_id);
        }
    }

    /// 合并 Partial<Message>，畸形补丁保留旧缓存，尚未加载的消息不造占位消息。
    pub fn patch(
        &mut self,
        room_id: i64,
        message_id: &str,
        patch: &Value,
    ) -> Result<bool, serde_json::Error> {
        let Some(room) = self.rooms.get_mut(&room_id) else {
            return Ok(false);
        };
        let Some(cached) =
            room.messages.iter_mut().find(|cached| cached.message.msg_id == message_id)
        else {
            return Ok(false);
        };
        cached.message.apply_patch(patch)?;
        self.payload_bytes -= cached.payload_bytes;
        cached.payload_bytes = message_size(&cached.message);
        self.payload_bytes += cached.payload_bytes;
        room.messages.sort_by_key(|cached| (cached.message.time, cached.sequence));
        self.enforce_limits();
        Ok(true)
    }

    /// deleteMessage 和 renewMessage(deleted=true) 共用去重；首次观察到撤回时返回 true。
    pub fn mark_deleted(&mut self, message_id: &str) -> bool {
        let first = self.remember_deleted(message_id);
        for room in self.rooms.values_mut() {
            for cached in &mut room.messages {
                if cached.message.msg_id == message_id {
                    let _ = cached.message.apply_patch(&json!({"deleted": true, "reveal": false}));
                    self.payload_bytes -= cached.payload_bytes;
                    cached.payload_bytes = message_size(&cached.message);
                    self.payload_bytes += cached.payload_bytes;
                }
            }
        }
        self.enforce_limits();
        first
    }

    /// URL 推送没有 roomId；只修改主附件及与其旧 URL 相同的媒体，保留其他附件。
    pub fn renew_url(&mut self, message_id: &str, url: &str) -> Vec<(i64, Value)> {
        let mut patches = Vec::new();
        for (room_id, room) in &mut self.rooms {
            let Some(cached) =
                room.messages.iter_mut().find(|cached| cached.message.msg_id == message_id)
            else {
                continue;
            };
            let mut value = cached.message.to_bridge_value();
            let old_url = value
                .get("file")
                .and_then(|file| file.get("url"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    value
                        .get("files")
                        .and_then(Value::as_array)
                        .and_then(|files| files.last())
                        .and_then(|file| file.get("url"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            let Some(old_url) = old_url else { continue };
            let mut patch = json!({});
            if let Some(file) = value.get_mut("file").filter(|file| file.is_object()) {
                file["url"] = json!(url);
                patch["file"] = file.clone();
            }
            if let Some(files) = value.get_mut("files").and_then(Value::as_array_mut) {
                for file in files.iter_mut() {
                    if file.get("url").and_then(Value::as_str) == Some(old_url.as_str()) {
                        file["url"] = json!(url);
                    }
                }
                patch["files"] = json!(files);
            }
            if cached.message.apply_patch(&patch).is_ok() {
                self.payload_bytes -= cached.payload_bytes;
                cached.payload_bytes = message_size(&cached.message);
                self.payload_bytes += cached.payload_bytes;
                patches.push((*room_id, patch));
            }
        }
        self.enforce_limits();
        patches
    }

    pub fn clear(&mut self) {
        self.rooms.clear();
        self.payload_bytes = 0;
        self.clock = 0;
        self.deleted_ids.clear();
        self.deleted_order.clear();
        self.deleted_id_bytes = 0;
    }

    fn remember_deleted(&mut self, message_id: &str) -> bool {
        if self.deleted_ids.contains(message_id) {
            return false;
        }
        // 一条超出整个去重预算的 ID 仍能通知插件，但不常驻缓存。
        if message_id.len() > DELETED_ID_BYTES {
            return true;
        }
        self.deleted_ids.insert(message_id.to_owned());
        self.deleted_order.push_back(message_id.to_owned());
        self.deleted_id_bytes += message_id.len();
        while self.deleted_order.len() > DELETED_IDS || self.deleted_id_bytes > DELETED_ID_BYTES {
            if let Some(id) = self.deleted_order.pop_front() {
                self.deleted_id_bytes -= id.len();
                self.deleted_ids.remove(&id);
            }
        }
        true
    }

    fn remove_room(&mut self, room_id: i64) {
        if let Some(room) = self.rooms.remove(&room_id) {
            self.payload_bytes -=
                room.messages.iter().map(|cached| cached.payload_bytes).sum::<usize>();
        }
    }

    fn enforce_limits(&mut self) {
        while self.rooms.len() > self.max_rooms {
            if let Some(room_id) = self.least_recent_room() {
                self.remove_room(room_id);
            }
        }
        while self.payload_bytes > self.max_payload_bytes {
            let Some(room_id) = self.least_recent_room() else {
                break;
            };
            let room = self.rooms.get_mut(&room_id).expect("缓存房间已存在");
            self.payload_bytes -= room.messages.remove(0).payload_bytes;
            if room.messages.is_empty() {
                self.rooms.remove(&room_id);
            }
        }
    }

    fn least_recent_room(&self) -> Option<i64> {
        self.rooms
            .iter()
            .min_by_key(|(room_id, room)| (room.last_access, **room_id))
            .map(|(room_id, _)| *room_id)
    }
}

fn message_size(message: &Message) -> usize {
    serde_json::to_vec(&message.to_bridge_value()).map_or(usize::MAX, |value| value.len())
}
