use crate::MAIN_STATUS;
use crate::config::BotConfig;

#[derive(Debug, Clone)]
pub struct BotStatus {
    pub config: Option<BotConfig>,
    pub ica_status: Option<ica::MainStatus>,
    pub tailchat_status: Option<tailchat::MainStatus>,
}

impl BotStatus {
    pub fn update_static_config(config: BotConfig) {
        unsafe {
            MAIN_STATUS.config = Some(config);
        }
    }
    pub fn update_ica_status(status: ica::MainStatus) {
        unsafe {
            MAIN_STATUS.ica_status = Some(status);
        }
    }
    pub fn update_tailchat_status(status: tailchat::MainStatus) {
        unsafe {
            MAIN_STATUS.tailchat_status = Some(status);
        }
    }

    pub fn static_init(config: BotConfig) {
        unsafe {
            MAIN_STATUS.ica_status = Some(ica::MainStatus {
                enable: config.check_ica(),
                qq_login: false,
                current_loaded_messages_count: 0,
                rooms: Vec::new(),
                online_status: ica::OnlineData::default(),
            });
            MAIN_STATUS.config = Some(config);
        }
    }

    pub fn global_config() -> &'static BotConfig {
        unsafe {
            let ptr = &raw const MAIN_STATUS.config;
            (*ptr).as_ref().unwrap()
        }
    }

    pub fn global_ica_status() -> &'static ica::MainStatus {
        unsafe {
            let ptr = &raw const MAIN_STATUS.ica_status;
            (*ptr).as_ref().unwrap()
        }
    }
    pub fn global_tailchat_status() -> &'static tailchat::MainStatus {
        unsafe {
            let ptr = &raw const MAIN_STATUS.tailchat_status;
            (*ptr).as_ref().unwrap()
        }
    }

    pub fn global_ica_status_mut() -> &'static mut ica::MainStatus {
        unsafe {
            let ptr = &raw mut MAIN_STATUS.ica_status;
            (*ptr).as_mut().unwrap()
        }
    }
    pub fn global_tailchat_status_mut() -> &'static mut tailchat::MainStatus {
        unsafe {
            let ptr = &raw mut MAIN_STATUS.tailchat_status;
            (*ptr).as_mut().unwrap()
        }
    }
}

pub mod ica {
    use crate::data_struct::ica::all_rooms::Room;
    pub use crate::data_struct::ica::online_data::OnlineData;

    #[derive(Debug, Clone)]
    pub struct MainStatus {
        /// 是否启用 ica
        pub enable: bool,
        /// qq 是否登录
        pub qq_login: bool,
        /// 当前已加载的消息数量
        pub current_loaded_messages_count: u64,
        /// 房间数据
        pub rooms: Vec<Room>,
        /// 在线数据 (Icalingua 信息)
        pub online_status: OnlineData,
    }

    impl MainStatus {
        pub fn update_rooms(&mut self, room: Vec<Room>) { self.rooms = room; }
        pub fn update_online_status(&mut self, status: OnlineData) { self.online_status = status; }
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
        pub fn update_user_id(&mut self, user_id: UserId) { self.user_id = user_id; }
        pub fn update_nick_name(&mut self, nick_name: String) { self.nick_name = nick_name; }
        pub fn update_email(&mut self, email: String) { self.email = email; }
        pub fn update_jwt_token(&mut self, jwt_token: String) { self.jwt_token = jwt_token; }
        pub fn update_avatar(&mut self, avatar: String) { self.avatar = avatar; }
    }
}
