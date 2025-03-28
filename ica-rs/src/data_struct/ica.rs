pub mod files;
pub mod messages;

pub mod all_rooms;
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
    fn as_room_id(&self) -> RoomId;
    fn as_chat_id(&self) -> RoomId;
}

impl RoomIdTrait for RoomId {
    fn is_room(&self) -> bool { (*self).is_negative() }
    fn as_room_id(&self) -> RoomId { -(*self).abs() }
    fn as_chat_id(&self) -> RoomId { (*self).abs() }
}
