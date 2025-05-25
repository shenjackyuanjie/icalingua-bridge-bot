pub mod events_func {

    /// icalingua 的 加群请求
    ///
    /// added: ica 2.0.1
    pub const ICA_JOIN_REQUEST: &str = "on_ica_join_request";
    /// icalingua 的 新消息
    pub const ICA_NEW_MESSAGE: &str = "on_ica_message";
    /// icalingua 的 消息撤回
    pub const ICA_DELETE_MESSAGE: &str = "on_ica_delete_message";

    /// tailchat 的 新消息
    pub const TAILCHAT_NEW_MESSAGE: &str = "on_tailchat_message";

    /// 加载时的事件
    ///
    /// added: bot 0.9.0
    pub const ON_LOAD: &str = "on_load";
    /// 卸载时的事件
    ///
    /// added: bot 0.9.0
    pub const ON_UNLOAD: &str = "on_unload";
    /// 重载时的事件
    ///
    /// added: bot 0.9.0
    pub const ON_RELOAD: &str = "on_reload";
}

/// 华丽的弃用了
///
/// 然后反悔, 还是得单独做出来
pub mod config_func {
    /// 请求配置用的函数
    pub const REQUIRE_CONFIG: &str = "require_config";
    /// 接受配置用的函数
    pub const ON_CONFIG: &str = "on_config";
}
