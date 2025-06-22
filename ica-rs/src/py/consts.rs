/// icalingua 的 事件函数
pub mod ica_func {
    /// icalingua 的 加群请求
    ///
    /// added: ica 2.0.1
    pub const JOIN_REQUEST: &str = "on_ica_join_request";
    /// icalingua 的 退群通知
    ///
    /// added: ica 2.0.1
    pub const LEAVE_MESSAGE: &str = "on_ica_leave_message";
    /// icalingua 的 新消息
    pub const NEW_MESSAGE: &str = "on_ica_message";
    /// icalingua 的 消息撤回
    pub const DELETE_MESSAGE: &str = "on_ica_delete_message";
    /// icalingua 的 系统消息
    ///
    /// added: ica 2.0.1
    pub const SYSTEM_MESSAGE: &str = "on_ica_system_message";
}

/// tailchat 的 事件函数
pub mod tailchat_func {
    /// 新消息
    pub const NEW_MESSAGE: &str = "on_tailchat_message";
}

/// 系统事件
pub mod sys_func {
    /// 加载时的事件
    //
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
    /// Mainfest
    ///
    /// added: bot 0.9.0
    pub const MANIFEST: &str = "PLUGIN_MANIFEST";
}
