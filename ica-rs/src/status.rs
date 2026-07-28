//! 机器人配置及各后端运行状态的全局访问接口。

use crate::MAIN_STATUS;
use crate::config::BotConfig;

#[derive(Debug, Clone)]
pub struct BotStatus {
    pub config: Option<BotConfig>,
    pub ica_status: Option<ica::MainStatus>,
    pub tailchat_status: Option<tailchat::MainStatus>,
}

impl BotStatus {
    /// 更新 `static_config` 状态。
    pub fn update_static_config(config: BotConfig) {
        unsafe {
            MAIN_STATUS.config = Some(config);
        }
    }
    /// 更新 `ica_status` 状态。
    pub fn update_ica_status(status: ica::MainStatus) {
        unsafe {
            MAIN_STATUS.ica_status = Some(status);
        }
    }
    /// 更新 `tailchat_status` 状态。
    pub fn update_tailchat_status(status: tailchat::MainStatus) {
        unsafe {
            MAIN_STATUS.tailchat_status = Some(status);
        }
    }

    /// 使用配置初始化全局运行状态。
    pub fn static_init(config: BotConfig) {
        unsafe {
            MAIN_STATUS.ica_status = Some(ica::MainStatus {
                enable: config.check_ica(),
                qq_login: false,
                loaded_messages_count_by_room: std::collections::HashMap::new(),
                rooms: Vec::new(),
                online_status: ica::OnlineData::default(),
            });
            MAIN_STATUS.config = Some(config);
        }
    }

    /// 返回全局机器人配置。
    pub fn global_config() -> &'static BotConfig {
        unsafe {
            let ptr = &raw const MAIN_STATUS.config;
            (*ptr).as_ref().unwrap()
        }
    }

    /// 返回全局 Icalingua 状态。
    pub fn global_ica_status() -> &'static ica::MainStatus {
        unsafe {
            let ptr = &raw const MAIN_STATUS.ica_status;
            (*ptr).as_ref().unwrap()
        }
    }
    /// 返回全局 Tailchat 状态。
    pub fn global_tailchat_status() -> &'static tailchat::MainStatus {
        unsafe {
            let ptr = &raw const MAIN_STATUS.tailchat_status;
            (*ptr).as_ref().unwrap()
        }
    }

    /// 返回可修改的全局 Icalingua 状态。
    pub fn global_ica_status_mut() -> &'static mut ica::MainStatus {
        unsafe {
            let ptr = &raw mut MAIN_STATUS.ica_status;
            (*ptr).as_mut().unwrap()
        }
    }
    /// 返回可修改的全局 Tailchat 状态。
    pub fn global_tailchat_status_mut() -> &'static mut tailchat::MainStatus {
        unsafe {
            let ptr = &raw mut MAIN_STATUS.tailchat_status;
            (*ptr).as_mut().unwrap()
        }
    }
}

pub mod ica {
    use std::collections::HashMap;

    use crate::data_struct::ica::all_rooms::Room;
    pub use crate::data_struct::ica::online_data::OnlineData;

    #[derive(Debug, Clone)]
    pub struct MainStatus {
        /// 是否启用 ica
        pub enable: bool,
        /// qq 是否登录
        pub qq_login: bool,
        /// 每个房间最近一次成功 `setMessages` 的消息数量。
        pub loaded_messages_count_by_room: HashMap<i64, u64>,
        /// 房间数据
        pub rooms: Vec<Room>,
        /// 在线数据 (Icalingua 信息)
        pub online_status: OnlineData,
    }

    impl MainStatus {
        /// 更新 `rooms` 状态。
        pub fn update_rooms(&mut self, room: Vec<Room>) {
            self.loaded_messages_count_by_room
                .retain(|room_id, _| room.iter().any(|current| current.room_id == *room_id));
            self.rooms = room;
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

        /// 返回指定房间最近一次成功加载的消息数量。
        pub fn loaded_messages_count_for(&self, room_id: i64) -> u64 {
            self.loaded_messages_count_by_room.get(&room_id).copied().unwrap_or(0)
        }

        /// 清理上一条 ICA 连接留下的历史计数。
        pub fn clear_loaded_message_counts(&mut self) {
            self.loaded_messages_count_by_room.clear();
        }
    }

    #[cfg(test)]
    mod tests {
        use std::collections::HashMap;

        use super::{MainStatus, OnlineData};

        #[test]
        fn loaded_message_counts_are_per_room_saturating_and_clearable() {
            let mut status = MainStatus {
                enable: true,
                qq_login: false,
                loaded_messages_count_by_room: HashMap::from([(1, u64::MAX), (2, 10)]),
                rooms: Vec::new(),
                online_status: OnlineData::default(),
            };
            assert_eq!(status.loaded_messages_count(), u64::MAX);
            assert_eq!(status.loaded_messages_count_for(2), 10);
            assert_eq!(status.loaded_messages_count_for(3), 0);
            status.clear_loaded_message_counts();
            assert_eq!(status.loaded_messages_count(), 0);
        }
    }
}

pub mod tailchat {
    use crate::data_struct::tailchat::UserId;

    #[derive(Debug, Clone)]
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
