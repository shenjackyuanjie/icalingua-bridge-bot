//! 暴露给 Python 插件的 Icalingua 消息、房间和客户端类型。

use std::time::SystemTime;

use pyo3::{PyResult, Python, exceptions::PyRuntimeError, pyclass, pymethods};
use rust_socketio::asynchronous::Client;

use tracing::{Level, event};

use crate::data_struct::ica::group_members::GroupMember;
use crate::data_struct::ica::messages::raw::RawSendMessage;
use crate::data_struct::ica::messages::{
    DeleteMessage, MessageTrait, NewMessage, ReplyMessage, SendMessage,
};
use crate::data_struct::ica::{MessageId, RoomId, RoomIdTrait, UserId, all_rooms};
use crate::ica::client::{
    delete_message, get_group_members, get_muted_group_members, get_nt_pic_url_by_file_id,
    handle_request, report_read, send_message, send_poke, send_room_sign_in, send_string_message,
    set_group_ban,
};
use crate::py::{binding::RuntimeBinding, host::PluginHost, storage};
use std::sync::Arc;

#[pyclass]
#[pyo3(name = "IcaStatus")]
pub struct IcaStatusPy {
    binding: RuntimeBinding,
}

#[pymethods]
impl IcaStatusPy {
    #[new]
    /// 构造供 Python 调用的新实例。
    pub fn py_new(py: Python<'_>) -> PyResult<Self> {
        Ok(Self {
            binding: RuntimeBinding::from_module(py)?,
        })
    }
    #[getter]
    /// 返回 `qq_login` 对应的数据。
    pub fn get_qq_login(&self, py: Python<'_>) -> bool {
        py.detach(|| self.binding.context.read_ica(|state| state.qq_login))
    }
    #[getter]
    /// 返回 `online` 对应的数据。
    pub fn get_online(&self, py: Python<'_>) -> bool {
        py.detach(|| self.binding.context.read_ica(|state| state.online_status.online))
    }
    #[getter]
    /// 返回 `self_id` 对应的数据。
    pub fn get_self_id(&self, py: Python<'_>) -> i64 {
        py.detach(|| self.binding.context.read_ica(|state| state.online_status.qqid))
    }
    #[getter]
    /// 返回 `nick_name` 对应的数据。
    pub fn get_nick_name(&self, py: Python<'_>) -> String {
        py.detach(|| self.binding.context.read_ica(|state| state.online_status.nick.clone()))
    }
    #[getter]
    /// 返回 `loaded_messages_count` 对应的数据。
    pub fn get_loaded_messages_count(&self, py: Python<'_>) -> u64 {
        py.detach(|| self.binding.context.read_ica(|state| state.loaded_messages_count()))
    }
    /// 返回指定房间最近一次成功加载的消息数量，未知房间返回 0。
    pub fn loaded_messages_count_for(&self, py: Python<'_>, room_id: RoomId) -> u64 {
        py.detach(|| {
            self.binding.context.read_ica(|state| state.loaded_messages_count_for(room_id))
        })
    }
    #[getter]
    /// 返回 `ica_version` 对应的数据。
    pub fn get_ica_version(&self, py: Python<'_>) -> String {
        py.detach(|| {
            self.binding
                .context
                .read_ica(|state| state.online_status.icalingua_info.ica_version.clone())
        })
    }

    #[getter]
    /// 返回 `os_info` 对应的数据。
    pub fn get_os_info(&self, py: Python<'_>) -> String {
        py.detach(|| {
            self.binding
                .context
                .read_ica(|state| state.online_status.icalingua_info.os_info.clone())
        })
    }

    #[getter]
    /// 返回 `resident_set_size` 对应的数据。
    pub fn get_resident_set_size(&self, py: Python<'_>) -> String {
        py.detach(|| {
            self.binding
                .context
                .read_ica(|state| state.online_status.icalingua_info.resident_set_size.clone())
        })
    }

    #[getter]
    /// 返回 `heap_used` 对应的数据。
    pub fn get_heap_used(&self, py: Python<'_>) -> String {
        py.detach(|| {
            self.binding
                .context
                .read_ica(|state| state.online_status.icalingua_info.heap_used.clone())
        })
    }

    #[getter]
    /// 返回 `load` 对应的数据。
    pub fn get_load(&self, py: Python<'_>) -> String {
        py.detach(|| {
            self.binding
                .context
                .read_ica(|state| state.online_status.icalingua_info.load.clone())
        })
    }
    #[getter]
    /// 获取当前用户加入的所有房间
    ///
    /// 添加自 2.0.1
    pub fn get_rooms(&self, py: Python<'_>) -> Vec<IcaRoomPy> {
        py.detach(|| {
            self.binding
                .context
                .read_ica(|state| state.rooms.iter().map(|r| r.into()).collect())
        })
    }
    #[getter]
    /// 获取所有管理员
    ///
    /// 添加自 2.0.1
    pub fn get_admins(&self) -> Vec<UserId> { self.binding.context.config.ica().admin_list.clone() }
    #[getter]
    /// 获取所有被屏蔽的人
    ///
    /// (好像没啥用就是了, 反正被过滤的不会给到插件)
    ///
    /// 添加自 2.0.1
    pub fn get_filtered(&self) -> Vec<UserId> {
        self.binding.context.config.ica().filter_list.clone()
    }
}

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

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "NewMessage")]
pub struct NewMessagePy {
    pub msg: NewMessage,
    context: Arc<crate::context::BotContext>,
}

#[pymethods]
impl NewMessagePy {
    /// 构造回复当前消息的新消息。
    pub fn reply_with(&self, content: String) -> SendMessagePy {
        SendMessagePy::new(self.msg.reply_with(&content))
    }
    /// 返回当前值的 `deleted` 表示。
    pub fn as_deleted(&self) -> DeleteMessagePy { DeleteMessagePy::new(self.msg.as_deleted()) }
    /// 返回适合 Python 展示的字符串。
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
    #[getter]
    /// 返回 `id` 对应的数据。
    pub fn get_id(&self) -> MessageId { self.msg.msg_id().clone() }
    #[getter]
    /// 返回 `content` 对应的数据。
    pub fn get_content(&self) -> String { self.msg.content().clone() }
    #[getter]
    /// 返回 `sender_id` 对应的数据。
    pub fn get_sender_id(&self) -> i64 { self.msg.sender_id() }
    #[getter]
    /// 返回 `sender_name` 对应的数据。
    pub fn get_sender_name(&self) -> String { self.msg.sender_name().clone() }
    #[getter]
    /// 返回 `is_from_self` 对应的数据。
    pub fn get_is_from_self(&self, py: Python<'_>) -> bool {
        py.detach(|| {
            self.context.read_ica(|status| self.msg.is_from_self(status.online_status.qqid))
        })
    }
    #[getter]
    /// 返回 `is_reply` 对应的数据。
    pub fn get_is_reply(&self) -> bool { self.msg.is_reply() }
    #[getter]
    /// 返回 `is_room_msg` 对应的数据。
    pub fn get_is_room_msg(&self) -> bool { self.msg.room_id.is_room() }
    #[getter]
    /// 返回 `is_chat_msg` 对应的数据。
    pub fn get_is_chat_msg(&self) -> bool { self.msg.room_id.is_chat() }
    #[getter]
    /// 返回 `room_id` 对应的数据。
    pub fn get_room_id(&self) -> RoomId { self.msg.room_id }
    /// reply message id
    ///
    /// 添加自 2.0.2
    #[getter]
    pub fn get_reply_msg_id(&self) -> Option<MessageId> {
        self.msg.msg.reply.as_ref().map(|r| r.msg_id.clone())
    }
    /// reply message content
    ///
    /// 添加自 2.0.2
    #[getter]
    pub fn get_reply_msg_content(&self) -> Option<String> {
        self.msg.msg.reply.as_ref().map(|r| r.content.clone())
    }
    /// reply message sender name
    ///
    /// 添加自 2.0.2
    #[getter]
    pub fn get_reply_msg_sender_name(&self) -> Option<String> {
        self.msg.msg.reply.as_ref().map(|r| r.sender_name.clone())
    }
}

impl NewMessagePy {
    /// 创建并初始化对应的数据结构。
    pub fn new(msg: &NewMessage, context: &Arc<crate::context::BotContext>) -> Self {
        Self {
            msg: msg.clone(),
            context: context.clone(),
        }
    }
}

#[pyclass]
#[pyo3(name = "ReplyMessage")]
pub struct ReplyMessagePy {
    pub msg: ReplyMessage,
}

#[pymethods]
impl ReplyMessagePy {
    /// 返回适合 Python 展示的字符串。
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
}

impl ReplyMessagePy {
    /// 创建并初始化对应的数据结构。
    pub fn new(msg: ReplyMessage) -> Self { Self { msg } }
}

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "SendMessage")]
pub struct SendMessagePy {
    pub msg: SendMessage,
}

#[pymethods]
impl SendMessagePy {
    /// 返回适合 Python 展示的字符串。
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
    /// 设置消息内容
    /// 用于链式调用
    pub fn with_content(&mut self, content: String) -> Self {
        self.msg.content = content;
        self.clone()
    }
    #[getter]
    /// 返回 `content` 对应的数据。
    pub fn get_content(&self) -> String { self.msg.content.clone() }
    #[setter]
    /// 更新 `content` 对应的数据。
    pub fn set_content(&mut self, content: String) { self.msg.content = content; }
    #[getter]
    /// 返回 `room_id` 对应的数据。
    pub fn get_room_id(&self) -> RoomId { self.msg.room_id }
    #[setter]
    /// 更新 `room_id` 对应的数据。
    pub fn set_room_id(&mut self, room_id: RoomId) { self.msg.room_id = room_id; }
    /// 设置消息图片
    pub fn set_img(&mut self, file: Vec<u8>, file_type: String, as_sticker: bool) {
        self.msg.set_img(&file, &file_type, as_sticker);
    }
    /// 追加一张 Base64 图片/媒体。
    pub fn add_img(&mut self, file: Vec<u8>, file_type: String) {
        self.msg.add_img(&file, &file_type);
    }
    /// 设置语音；不在 bot 端录音或转码，失败时保留已有媒体。
    pub fn set_audio(&mut self, file: Vec<u8>, file_type: String) -> PyResult<()> {
        self.msg.set_audio(&file, &file_type).map_err(PyRuntimeError::new_err)
    }
    /// 复用 Bridge 下发的语音资源 ID。
    pub fn set_audio_fid(&mut self, fid: String, file_type: String) -> PyResult<()> {
        self.msg.set_audio_fid(&fid, &file_type).map_err(PyRuntimeError::new_err)
    }
    /// 移除消息回复引用。
    pub fn remove_reply(&mut self) -> Self {
        self.msg.reply_to = None;
        self.clone()
    }
}

impl SendMessagePy {
    /// 创建并初始化对应的数据结构。
    pub fn new(msg: SendMessage) -> Self { Self { msg } }
}

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "DeleteMessage")]
pub struct DeleteMessagePy {
    pub msg: DeleteMessage,
}

#[pymethods]
impl DeleteMessagePy {
    /// 返回适合 Python 展示的字符串。
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
}

impl DeleteMessagePy {
    /// 创建并初始化对应的数据结构。
    pub fn new(msg: DeleteMessage) -> Self { Self { msg } }
}

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "IcaClient")]
pub struct IcaClientPy {
    pub client: Client,
    binding: RuntimeBinding,
}

#[pymethods]
impl IcaClientPy {
    /// 主动上报 QQ 已读，失败时抛出 RuntimeError。
    pub fn report_read(&self, py: Python<'_>, message_id: String) -> PyResult<()> {
        self.binding.with_host(py, |host| {
            let rt = &host.context.runtime;
            rt.block_on(report_read(&self.client, &message_id))
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// 批准或拒绝好友/群申请；成功只表示请求已发送。
    #[pyo3(signature = (request_type, flag, accept = true))]
    pub fn handle_request(
        &self,
        py: Python<'_>,
        request_type: String,
        flag: String,
        accept: bool,
    ) -> PyResult<()> {
        self.binding.with_host(py, |host| {
            let rt = &host.context.runtime;
            rt.block_on(handle_request(&self.client, &request_type, &flag, accept))
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// 请求 QQ NT 图片新地址；12 秒 ACK 超时或无效地址会抛出 RuntimeError。
    #[pyo3(signature = (file_id, app_id = "".to_string()))]
    pub fn get_nt_pic_url_by_file_id(
        &self,
        py: Python<'_>,
        file_id: String,
        app_id: String,
    ) -> PyResult<String> {
        self.binding.with_host(py, |host| {
            let rt = &host.context.runtime;
            rt.block_on(get_nt_pic_url_by_file_id(&self.client, &file_id, &app_id))
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// 签到
    ///
    /// 添加自 1.6.5 版本
    pub fn send_room_sign_in(&self, py: Python<'_>, room_id: RoomId) -> bool {
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(send_room_sign_in(&self.client, room_id))
            })
            .unwrap_or(false)
    }

    /// 戳一戳
    ///
    /// 添加自 1.6.5 版本
    pub fn send_poke(&self, py: Python<'_>, room_id: RoomId, user_id: UserId) -> bool {
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(send_poke(&self.client, room_id, user_id))
            })
            .unwrap_or(false)
    }

    /// 禁言指定群成员
    ///
    /// duration 单位为秒，设为 0 时解除禁言，最大为 30 天。
    pub fn set_group_ban(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        user_id: UserId,
        duration: u64,
    ) -> bool {
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(set_group_ban(&self.client, room_id, user_id, duration))
            })
            .unwrap_or(false)
    }

    /// 获取指定群聊的完整成员列表。
    pub fn get_group_members(
        &self,
        py: Python<'_>,
        room_id: RoomId,
    ) -> PyResult<Vec<IcaGroupMemberPy>> {
        self.binding.with_host(py, |host| {
            let rt = &host.context.runtime;
            rt.block_on(get_group_members(&self.client, room_id))
                .map(|members| members.into_iter().map(Into::into).collect())
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))
        })?
    }

    /// 获取指定群聊中当前仍处于禁言中的成员。
    pub fn get_muted_group_members(
        &self,
        py: Python<'_>,
        room_id: RoomId,
    ) -> PyResult<Vec<IcaGroupMemberPy>> {
        self.binding.with_host(py, |host| {
            let rt = &host.context.runtime;
            rt.block_on(get_muted_group_members(&self.client, room_id))
                .map(|members| members.into_iter().map(Into::into).collect())
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))
        })?
    }

    /// 发送 `message` 请求或消息。
    pub fn send_message(&self, py: Python<'_>, message: SendMessagePy) -> bool {
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(send_message(&self.client, &message.msg, &host.context))
            })
            .unwrap_or(false)
    }

    /// 发送一条 raw 的消息
    ///
    /// 懒得做 serde+deser 了, 就干脆传 string
    ///
    /// # WARN: 小心使用
    ///
    /// 添加自: 2.0.1 版本
    pub fn send_raw_message(&self, py: Python<'_>, raw_msg: String, room_id: RoomId) -> bool {
        let msg = RawSendMessage::string_to_json(&raw_msg, room_id);
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(send_string_message(&self.client, &msg, &host.context))
            })
            .unwrap_or(false)
    }

    /// 发送 `and_warn` 请求或消息。
    pub fn send_and_warn(&self, py: Python<'_>, message: SendMessagePy) -> bool {
        event!(Level::WARN, message.msg.content);
        self.send_message(py, message)
    }

    /// 请求删除指定消息。
    pub fn delete_message(&self, py: Python<'_>, message: DeleteMessagePy) -> bool {
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(delete_message(&self.client, &message.msg))
            })
            .unwrap_or(false)
    }

    /// 直接从参数撤回消息
    ///
    /// 额…… 我才发现之前的那个 api 几乎没法用
    ///
    /// 私密马赛
    ///
    /// 添加自: 2.0.2 版本
    pub fn delete_msg_raw(&self, py: Python<'_>, room_id: RoomId, msg_id: MessageId) -> bool {
        let msg = DeleteMessage::new(room_id, msg_id);
        self.binding
            .with_host(py, |host| {
                let rt = &host.context.runtime;
                rt.block_on(delete_message(&self.client, &msg))
            })
            .unwrap_or(false)
    }

    #[getter]
    /// 返回 `status` 对应的数据。
    pub fn get_status(&self) -> IcaStatusPy {
        IcaStatusPy {
            binding: self.binding.clone(),
        }
    }
    #[getter]
    /// 返回 `version` 对应的数据。
    pub fn get_version(&self) -> String { crate::VERSION.to_string() }
    #[getter]
    /// 返回 `version_str` 对应的数据。
    pub fn get_version_str(&self) -> String { crate::version_str() }
    #[getter]
    /// 返回 `client_id` 对应的数据。
    pub fn get_client_id(&self) -> String { crate::client_id() }
    #[getter]
    /// 返回 `ica_version` 对应的数据。
    pub fn get_ica_version(&self) -> String { crate::ICA_VERSION.to_string() }
    #[getter]
    /// 返回 `startup_time` 对应的数据。
    pub fn get_startup_time(&self) -> SystemTime { crate::start_up_time() }

    #[getter]
    /// 返回 `py_tasks_count` 对应的数据。
    pub fn get_py_tasks_count(&self, py: Python<'_>) -> usize {
        self.binding.with_host(py, |host| host.tasks.total_len()).unwrap_or(0)
    }

    /// 重新加载插件状态
    /// 返回是否成功
    pub fn sync_status_from_file(&self, py: Python<'_>) -> PyResult<()> {
        self.binding
            .with_host(py, |host| {
                host.context.runtime.block_on(storage::sync_status_from_file(host))
            })?
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }

    /// 同步状态到配置文件
    /// 这样关闭的时候就会保存状态
    pub fn sync_status_to_file(&self, py: Python<'_>) -> PyResult<()> {
        self.binding
            .with_host(py, |host| {
                host.context.runtime.block_on(storage::sync_status_to_file(host))
            })?
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }

    /// 设置某个插件的状态
    pub fn set_plugin_status(
        &self,
        py: Python<'_>,
        plugin_name: String,
        status: bool,
    ) -> PyResult<()> {
        self.binding
            .with_host(py, |host| {
                host.context.runtime.block_on(storage::set_plugin_status(
                    host,
                    &plugin_name,
                    status,
                ))
            })?
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }

    /// 返回 `plugin_status` 对应的数据。
    pub fn get_plugin_status(&self, py: Python<'_>, plugin_name: String) -> Option<bool> {
        self.binding
            .with_host(py, |host| host.storage.blocking_lock().get_status(&plugin_name))
            .ok()
            .flatten()
    }

    /// 重新加载插件
    ///
    /// 返回是否成功
    pub fn reload_plugin(&self, py: Python<'_>, plugin_name: String) -> bool {
        self.binding
            .with_host(py, |host| {
                host.context
                    .runtime
                    .block_on(storage::reload_plugin(host, &plugin_name))
                    .is_ok()
            })
            .unwrap_or(false)
    }

    /// 向 Python 插件日志记录调试信息。
    pub fn debug(&self, content: String) {
        event!(Level::DEBUG, "{}", content);
    }
    /// 向 Python 插件日志记录普通信息。
    pub fn info(&self, content: String) {
        event!(Level::INFO, "{}", content);
    }
    /// 向 Python 插件日志记录警告信息。
    pub fn warn(&self, content: String) {
        event!(Level::WARN, "{}", content);
    }
}

impl IcaClientPy {
    /// 创建并初始化对应的数据结构。
    pub fn new(client: &Client, host: &Arc<PluginHost>) -> Self {
        Self {
            client: client.clone(),
            binding: RuntimeBinding::new(host),
        }
    }
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
