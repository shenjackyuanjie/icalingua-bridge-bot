//! 暴露给 Python 插件的 ICA 房间、群成员和加入请求。

use pyo3::{pyclass, pymethods};

use crate::data_struct::ica::{RoomId, RoomIdTrait, UserId, all_rooms, group_members::GroupMember};

use super::send::SendMessagePy;

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "IcaRoom")]
/// Room api
///
/// 添加自 2.0.1
pub struct IcaRoomPy {
    pub inner: crate::data_struct::ica::all_rooms::Room,
}

impl From<crate::data_struct::ica::all_rooms::Room> for IcaRoomPy {
    /// 将来源值转换为当前类型。
    fn from(inner: crate::data_struct::ica::all_rooms::Room) -> Self { Self { inner } }
}

impl From<&crate::data_struct::ica::all_rooms::Room> for IcaRoomPy {
    /// 将来源值转换为当前类型。
    fn from(inner: &crate::data_struct::ica::all_rooms::Room) -> Self {
        Self {
            inner: inner.clone(),
        }
    }
}

#[pymethods]
impl IcaRoomPy {
    #[getter]
    /// 返回 `room_id` 对应的数据。
    pub fn get_room_id(&self) -> i64 { self.inner.room_id }
    #[getter]
    /// 返回 `room_name` 对应的数据。
    pub fn get_room_name(&self) -> String { self.inner.room_name.clone() }
    #[getter]
    /// 返回 `unread_count` 对应的数据。
    pub fn get_unread_count(&self) -> u64 { self.inner.unread_count }
    #[getter]
    /// 返回 `priority` 对应的数据。
    pub fn get_priority(&self) -> u8 { self.inner.priority }
    #[getter]
    /// 返回 `utime` 对应的数据。
    pub fn get_utime(&self) -> i64 { self.inner.utime }
    /// 判断当前值是否满足 `group` 条件。
    pub fn is_group(&self) -> bool { self.inner.room_id.is_room() }
    /// 判断当前值是否满足 `chat` 条件。
    pub fn is_chat(&self) -> bool { self.inner.room_id.is_chat() }
    /// 创建并初始化对应的数据结构。
    pub fn new_message_to(&self, content: String) -> SendMessagePy {
        SendMessagePy::new(self.inner.new_message_to(content))
    }
}

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "IcaGroupMember")]
pub struct IcaGroupMemberPy {
    pub inner: GroupMember,
}

impl From<GroupMember> for IcaGroupMemberPy {
    fn from(inner: GroupMember) -> Self { Self { inner } }
}

#[pymethods]
impl IcaGroupMemberPy {
    #[getter]
    pub fn get_user_id(&self) -> i64 { self.inner.user_id }
    #[getter]
    pub fn get_nickname(&self) -> String { self.inner.nickname.clone() }
    #[getter]
    pub fn get_card(&self) -> String { self.inner.card.clone() }
    #[getter]
    pub fn get_remark(&self) -> String { self.inner.remark.clone() }
    #[getter]
    pub fn get_title(&self) -> String { self.inner.title.clone() }
    #[getter]
    pub fn get_level(&self) -> String { self.inner.level.clone() }
    #[getter]
    pub fn get_role(&self) -> String { self.inner.role.clone() }
    #[getter]
    pub fn get_shutup_time(&self) -> i64 { self.inner.shutup_time }
    pub fn display_name(&self) -> String { self.inner.display_name().to_string() }
    pub fn is_muted_at(&self, timestamp: i64) -> bool { self.inner.is_muted_at(timestamp) }
    pub fn is_muted(&self) -> bool { self.inner.is_muted() }
    pub fn remaining_mute_seconds_at(&self, timestamp: i64) -> u64 {
        self.inner.remaining_mute_seconds_at(timestamp)
    }
    pub fn remaining_mute_seconds(&self) -> u64 { self.inner.remaining_mute_seconds() }
}

#[pyclass]
#[pyo3(name = "IcaJoinRequest")]
pub struct IcaJoinRequestPy {
    pub inner: all_rooms::JoinRequestRoom,
}

impl IcaJoinRequestPy {
    /// 创建并初始化对应的数据结构。
    pub fn new(event: &all_rooms::JoinRequestRoom) -> Self {
        Self {
            inner: event.clone(),
        }
    }
}

#[pymethods]
impl IcaJoinRequestPy {
    #[getter]
    /// 返回 `comment` 对应的数据。
    pub fn get_comment(&self) -> String { self.inner.comment.clone() }
    #[getter]
    /// 返回 `group_id` 对应的数据。
    pub fn get_group_id(&self) -> Option<RoomId> { self.inner.group_id }
    #[getter]
    /// 返回 `group_name` 对应的数据。
    pub fn get_group_name(&self) -> String { self.inner.group_name.clone() }
    #[getter]
    /// 返回 `user_id` 对应的数据。
    pub fn get_user_id(&self) -> UserId { self.inner.user_id }
    #[getter]
    /// 返回 `nickname` 对应的数据。
    pub fn get_nickname(&self) -> String { self.inner.nickname.clone() }
    #[getter]
    /// 返回 `request_type` 对应的数据。
    pub fn get_request_type(&self) -> String { self.inner.request_type.clone() }
    #[getter]
    /// 返回 `post_type` 对应的数据。
    pub fn get_post_type(&self) -> String { self.inner.post_type.clone() }
    #[getter]
    /// 返回 `sub_type` 对应的数据。
    pub fn get_sub_type(&self) -> String { self.inner.sub_type.clone() }
    #[getter]
    /// 返回 `time` 对应的数据。
    pub fn get_time(&self) -> i64 { self.inner.time }
    #[getter]
    /// 返回 `tips` 对应的数据。
    pub fn get_tips(&self) -> String { self.inner.tips.clone() }
    #[getter]
    /// 返回 `flag` 对应的数据。
    pub fn get_flag(&self) -> String { self.inner.flag.clone() }
    #[getter]
    /// 好友申请来源，缺失时为空字符串。
    pub fn get_source(&self) -> String { self.inner.source.clone() }
    #[getter]
    pub fn get_self_id(&self) -> Option<UserId> { self.inner.self_id }
    #[getter]
    pub fn get_age(&self) -> Option<u32> { self.inner.age }
    #[getter]
    pub fn get_sex(&self) -> Option<String> { self.inner.sex.clone() }
}
