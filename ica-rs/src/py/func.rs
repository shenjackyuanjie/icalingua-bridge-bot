pub mod events_func {

    /// icalingua 的 加群请求
    ///
    /// added: 2.0.1
    pub const ICA_JOIN_REQUEST: &str = "on_ica_join_request";
    /// icalingua 的 新消息
    pub const ICA_NEW_MESSAGE: &str = "on_ica_message";
    /// icalingua 的 消息撤回
    pub const ICA_DELETE_MESSAGE: &str = "on_ica_delete_message";

    /// tailchat 的 新消息
    pub const TAILCHAT_NEW_MESSAGE: &str = "on_tailchat_message";
}

pub mod config_func {
    /// 请求配置用的函数
    pub const REQUIRE_CONFIG: &str = "require_config";
    /// 接受配置用的函数
    pub const ON_CONFIG: &str = "on_config";
}
