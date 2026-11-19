//! Icalingua bridge 协议使用的数据结构及标识类型。

/// 加载 `files` 子模块。
pub mod files;
/// 加载群成员资料子模块。
pub mod group_members;
/// 加载 `messages` 子模块。
pub mod messages;

/// 加载 `all_rooms` 子模块。
pub mod all_rooms;
/// 加载 `online_data` 子模块。
pub mod online_data;

/// 房间 id
/// 群聊 < 0
/// 私聊 > 0
pub type RoomId = i64;
pub type UserId = i64;
pub type MessageId = String;

#[allow(unused)]
pub trait RoomIdTrait {
    /// 判断是否是群聊
    fn is_room(&self) -> bool;
    /// 判断是否是私聊
    fn is_chat(&self) -> bool { !self.is_room() }
    /// 返回当前值的 `room_id` 表示。
    fn as_room_id(&self) -> RoomId;
    /// 返回当前值的 `chat_id` 表示。
    fn as_chat_id(&self) -> RoomId;
}

impl RoomIdTrait for RoomId {
    /// 判断当前值是否满足 `room` 条件。
    fn is_room(&self) -> bool { (*self).is_negative() }
    /// 返回当前值的 `room_id` 表示。
    fn as_room_id(&self) -> RoomId { -(*self).abs() }
    /// 返回当前值的 `chat_id` 表示。
    fn as_chat_id(&self) -> RoomId { (*self).abs() }
}
