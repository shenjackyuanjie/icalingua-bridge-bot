//! 各后端运行状态；所有权和同步由 BotContext 管理。

#[cfg(test)]
#[path = "status/tests/ica.rs"]
mod tests;

pub mod ica {
    use std::collections::{HashMap, HashSet};

    use serde_json::{Value, json};

    use crate::data_struct::ica::all_rooms::Room;
    use crate::data_struct::ica::messages::{At, Message};
    pub use crate::data_struct::ica::online_data::OnlineData;
    use crate::ica::event_diagnostics::EventDiagnostics;
    use crate::ica::message_cache::LoadedMessagesCache;

    #[derive(Debug, Clone, Default)]
    pub struct MainStatus {
        /// 是否启用 ica
        pub enable: bool,
        /// qq 是否登录
        pub qq_login: bool,
        /// 每个房间实际保留的已加载消息数量。
        pub loaded_messages_count_by_room: HashMap<i64, u64>,
        /// 仅保存已经加载的消息，不主动抓取或常驻完整历史。
        pub loaded_messages: LoadedMessagesCache,
        /// 当前 Bot 实例的无效事件日志窗口，与聊天内容和缓存分开。
        pub event_diagnostics: EventDiagnostics,
        /// 房间数据
        pub rooms: Vec<Room>,
        /// 当前房间完整快照，保留 users、下载设置等尚无 typed 字段的数据。
        pub room_data: HashMap<i64, Value>,
        /// Bridge 聊天分组快照；与桌面通知分开消费。
        pub chat_groups: Vec<Value>,
        /// 在线数据 (Icalingua 信息)
        pub online_status: OnlineData,
    }

    impl MainStatus {
        /// 更新 `rooms` 状态。
        pub fn update_rooms(&mut self, rooms: Vec<Room>) {
            let room_ids: HashSet<_> = rooms.iter().map(|room| room.room_id).collect();
            self.loaded_messages.retain_rooms(&room_ids);
            self.loaded_messages_count_by_room
                .retain(|room_id, _| room_ids.contains(room_id));
            self.room_data.retain(|room_id, _| room_ids.contains(room_id));
            self.rooms = rooms;
        }

        /// 保存加载结果；历史和实时消息共用去重缓存，不触发 Python 新消息 hook。
        pub fn store_loaded_messages(
            &mut self,
            room_id: i64,
            messages: Vec<Message>,
            replace: bool,
        ) -> usize {
            let accepted = self.loaded_messages.store(room_id, messages, replace);
            self.sync_loaded_counts();
            accepted
        }

        pub fn get_loaded_messages(&self, room_id: i64, limit: usize) -> Vec<Message> {
            self.loaded_messages.get(room_id, limit)
        }

        pub fn patch_loaded_message(
            &mut self,
            room_id: i64,
            message_id: &str,
            patch: &Value,
        ) -> Result<bool, serde_json::Error> {
            let result = self.loaded_messages.patch(room_id, message_id, patch);
            self.sync_loaded_counts();
            result
        }

        pub fn mark_message_deleted(&mut self, message_id: &str) -> bool {
            let first = self.loaded_messages.mark_deleted(message_id);
            self.sync_loaded_counts();
            first
        }

        pub fn renew_loaded_message_url(
            &mut self,
            message_id: &str,
            url: &str,
        ) -> Vec<(i64, Value)> {
            let patches = self.loaded_messages.renew_url(message_id, url);
            self.sync_loaded_counts();
            patches
        }

        /// QQ 同步已读既清未读数量，也清 @ 与定位标记。
        pub fn sync_read(&mut self, room_id: i64) {
            if let Some(room) = self.rooms.iter_mut().find(|room| room.room_id == room_id) {
                room.unread_count = 0;
                room.at = At::Bool(false);
            }
            if let Some(room) = self.room_data.get_mut(&room_id).and_then(Value::as_object_mut) {
                room.insert("unreadCount".into(), json!(0));
                room.insert("at".into(), json!(false));
                room.insert("atMessageId".into(), Value::Null);
            }
        }

        fn sync_loaded_counts(&mut self) {
            self.loaded_messages_count_by_room = self.loaded_messages.counts();
        }
        /// 更新 `online_status` 状态。
        pub fn update_online_status(&mut self, status: OnlineData) { self.online_status = status; }

        /// 返回所有房间已加载消息数量的饱和总和。
        pub fn loaded_messages_count(&self) -> u64 {
            self.loaded_messages_count_by_room
                .values()
                .copied()
                .fold(0_u64, u64::saturating_add)
        }

        /// 返回指定房间实际保留的已加载消息数量。
        pub fn loaded_messages_count_for(&self, room_id: i64) -> u64 {
            self.loaded_messages_count_by_room.get(&room_id).copied().unwrap_or(0)
        }

        /// 清理上一条 ICA 连接留下的历史缓存、计数与撤回去重。
        pub fn clear_loaded_message_counts(&mut self) {
            self.loaded_messages.clear();
            self.loaded_messages_count_by_room.clear();
        }
    }
}

pub mod tailchat {
    use crate::data_struct::tailchat::UserId;

    #[derive(Debug, Clone, Default)]
    pub struct MainStatus {
        /// 是否启用 tailchat
        pub enable: bool,
        /// 是否登录
        pub login: bool,
        /// 用户 ID
        pub user_id: UserId,
        /// 昵称
        pub nick_name: String,
        /// 邮箱
        pub email: String,
        /// JWT Token
        pub jwt_token: String,
        /// avatar
        pub avatar: String,
    }

    impl MainStatus {
        /// 更新 `user_id` 状态。
        pub fn update_user_id(&mut self, user_id: UserId) { self.user_id = user_id; }
        /// 更新 `nick_name` 状态。
        pub fn update_nick_name(&mut self, nick_name: String) { self.nick_name = nick_name; }
        /// 更新 `email` 状态。
        pub fn update_email(&mut self, email: String) { self.email = email; }
        /// 更新 `jwt_token` 状态。
        pub fn update_jwt_token(&mut self, jwt_token: String) { self.jwt_token = jwt_token; }
        /// 更新 `avatar` 状态。
        pub fn update_avatar(&mut self, avatar: String) { self.avatar = avatar; }
    }
}
