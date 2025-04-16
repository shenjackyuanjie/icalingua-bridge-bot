use std::time::SystemTime;

use pyo3::{pyclass, pymethods};
use rust_socketio::asynchronous::Client;
use tokio::runtime::Runtime;
use tracing::{Level, event};

use crate::MainStatus;
use crate::data_struct::ica::messages::raw::RawSendMessage;
use crate::data_struct::ica::messages::{
    DeleteMessage, MessageTrait, NewMessage, ReplyMessage, SendMessage,
};
use crate::data_struct::ica::{MessageId, RoomId, RoomIdTrait, UserId};
use crate::ica::client::{
    delete_message, send_message, send_poke, send_room_sign_in, send_string_message,
};
use crate::py::PyStatus;

#[pyclass]
#[pyo3(name = "IcaStatus")]
pub struct IcaStatusPy {}

#[pymethods]
impl IcaStatusPy {
    #[new]
    pub fn py_new() -> Self { Self {} }
    #[getter]
    pub fn get_qq_login(&self) -> bool { MainStatus::global_ica_status().qq_login }
    #[getter]
    pub fn get_online(&self) -> bool { MainStatus::global_ica_status().online_status.online }
    #[getter]
    pub fn get_self_id(&self) -> i64 { MainStatus::global_ica_status().online_status.qqid }
    #[getter]
    pub fn get_nick_name(&self) -> String {
        MainStatus::global_ica_status().online_status.nick.clone()
    }
    #[getter]
    pub fn get_loaded_messages_count(&self) -> u64 {
        MainStatus::global_ica_status().current_loaded_messages_count
    }
    #[getter]
    pub fn get_ica_version(&self) -> String {
        MainStatus::global_ica_status().online_status.icalingua_info.ica_version.clone()
    }

    #[getter]
    pub fn get_os_info(&self) -> String {
        MainStatus::global_ica_status().online_status.icalingua_info.os_info.clone()
    }

    #[getter]
    pub fn get_resident_set_size(&self) -> String {
        MainStatus::global_ica_status()
            .online_status
            .icalingua_info
            .resident_set_size
            .clone()
    }

    #[getter]
    pub fn get_heap_used(&self) -> String {
        MainStatus::global_ica_status().online_status.icalingua_info.heap_used.clone()
    }

    #[getter]
    pub fn get_load(&self) -> String {
        MainStatus::global_ica_status().online_status.icalingua_info.load.clone()
    }
    #[getter]
    /// 获取当前用户加入的所有房间
    ///
    /// 添加自 2.0.1
    pub fn get_rooms(&self) -> Vec<IcaRoomPy> {
        MainStatus::global_ica_status().rooms.iter().map(|r| r.into()).collect()
    }
    #[getter]
    /// 获取所有管理员
    ///
    /// 添加自 2.0.1
    pub fn get_admins(&self) -> Vec<UserId> { MainStatus::global_config().ica().admin_list.clone() }
    #[getter]
    /// 获取所有被屏蔽的人
    ///
    /// (好像没啥用就是了, 反正被过滤的不会给到插件)
    ///
    /// 添加自 2.0.1
    pub fn get_filtered(&self) -> Vec<UserId> {
        MainStatus::global_config().ica().filter_list.clone()
    }
}

impl Default for IcaStatusPy {
    fn default() -> Self { Self::new() }
}

impl IcaStatusPy {
    pub fn new() -> Self { Self {} }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "IcaRoom")]
/// Room api
///
/// 添加自 2.0.1
pub struct IcaRoomPy {
    pub inner: crate::data_struct::ica::all_rooms::Room,
}

impl From<crate::data_struct::ica::all_rooms::Room> for IcaRoomPy {
    fn from(inner: crate::data_struct::ica::all_rooms::Room) -> Self { Self { inner } }
}

impl From<&crate::data_struct::ica::all_rooms::Room> for IcaRoomPy {
    fn from(inner: &crate::data_struct::ica::all_rooms::Room) -> Self {
        Self {
            inner: inner.clone(),
        }
    }
}

#[pymethods]
impl IcaRoomPy {
    #[getter]
    pub fn get_room_id(&self) -> i64 { self.inner.room_id }
    #[getter]
    pub fn get_room_name(&self) -> String { self.inner.room_name.clone() }
    #[getter]
    pub fn get_unread_count(&self) -> u64 { self.inner.unread_count }
    #[getter]
    pub fn get_priority(&self) -> u8 { self.inner.priority }
    #[getter]
    pub fn get_utime(&self) -> i64 { self.inner.utime }
    pub fn is_group(&self) -> bool { self.inner.room_id.is_room() }
    pub fn is_chat(&self) -> bool { self.inner.room_id.is_chat() }
    pub fn new_message_to(&self, content: String) -> SendMessagePy {
        SendMessagePy::new(self.inner.new_message_to(content))
    }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "NewMessage")]
pub struct NewMessagePy {
    pub msg: NewMessage,
}

#[pymethods]
impl NewMessagePy {
    pub fn reply_with(&self, content: String) -> SendMessagePy {
        SendMessagePy::new(self.msg.reply_with(&content))
    }
    pub fn as_deleted(&self) -> DeleteMessagePy { DeleteMessagePy::new(self.msg.as_deleted()) }
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
    #[getter]
    pub fn get_id(&self) -> MessageId { self.msg.msg_id().clone() }
    #[getter]
    pub fn get_content(&self) -> String { self.msg.content().clone() }
    #[getter]
    pub fn get_sender_id(&self) -> i64 { self.msg.sender_id() }
    #[getter]
    pub fn get_sender_name(&self) -> String { self.msg.sender_name().clone() }
    #[getter]
    pub fn get_is_from_self(&self) -> bool { self.msg.is_from_self() }
    #[getter]
    pub fn get_is_reply(&self) -> bool { self.msg.is_reply() }
    #[getter]
    pub fn get_is_room_msg(&self) -> bool { self.msg.room_id.is_room() }
    #[getter]
    pub fn get_is_chat_msg(&self) -> bool { self.msg.room_id.is_chat() }
    #[getter]
    pub fn get_room_id(&self) -> RoomId { self.msg.room_id }
}

impl NewMessagePy {
    pub fn new(msg: &NewMessage) -> Self { Self { msg: msg.clone() } }
}

#[pyclass]
#[pyo3(name = "ReplyMessage")]
pub struct ReplyMessagePy {
    pub msg: ReplyMessage,
}

#[pymethods]
impl ReplyMessagePy {
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
}

impl ReplyMessagePy {
    pub fn new(msg: ReplyMessage) -> Self { Self { msg } }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "SendMessage")]
pub struct SendMessagePy {
    pub msg: SendMessage,
}

#[pymethods]
impl SendMessagePy {
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
    /// 设置消息内容
    /// 用于链式调用
    pub fn with_content(&mut self, content: String) -> Self {
        self.msg.content = content;
        self.clone()
    }
    #[getter]
    pub fn get_content(&self) -> String { self.msg.content.clone() }
    #[setter]
    pub fn set_content(&mut self, content: String) { self.msg.content = content; }
    #[getter]
    pub fn get_room_id(&self) -> RoomId { self.msg.room_id }
    #[setter]
    pub fn set_room_id(&mut self, room_id: RoomId) { self.msg.room_id = room_id; }
    /// 设置消息图片
    pub fn set_img(&mut self, file: Vec<u8>, file_type: String, as_sticker: bool) {
        self.msg.set_img(&file, &file_type, as_sticker);
    }
    pub fn remove_reply(&mut self) -> Self {
        self.msg.reply_to = None;
        self.clone()
    }
}

impl SendMessagePy {
    pub fn new(msg: SendMessage) -> Self { Self { msg } }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "DeleteMessage")]
pub struct DeleteMessagePy {
    pub msg: DeleteMessage,
}

#[pymethods]
impl DeleteMessagePy {
    pub fn __str__(&self) -> String { format!("{:?}", self.msg) }
}

impl DeleteMessagePy {
    pub fn new(msg: DeleteMessage) -> Self { Self { msg } }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "IcaClient")]
pub struct IcaClientPy {
    pub client: Client,
}

#[pymethods]
impl IcaClientPy {
    /// 签到
    ///
    /// 添加自 1.6.5 版本
    pub fn send_room_sign_in(&self, room_id: RoomId) -> bool {
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(send_room_sign_in(&self.client, room_id))
        })
    }

    /// 戳一戳
    ///
    /// 添加自 1.6.5 版本
    pub fn send_poke(&self, room_id: RoomId, user_id: UserId) -> bool {
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(send_poke(&self.client, room_id, user_id))
        })
    }

    pub fn send_message(&self, message: SendMessagePy) -> bool {
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(send_message(&self.client, &message.msg))
        })
    }

    /// 发送一条 raw 的消息
    ///
    /// 懒得做 serde+deser 了, 就干脆传 string
    ///
    /// # WARN: 小心使用
    ///
    /// 添加自: 2.0.1 版本
    pub fn send_raw_message(&self, raw_msg: String, room_id: RoomId) -> bool {
        let msg = RawSendMessage::string_to_json(&raw_msg, room_id);
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(send_string_message(&self.client, msg))
        })
    }

    pub fn send_and_warn(&self, message: SendMessagePy) -> bool {
        event!(Level::WARN, message.msg.content);
        self.send_message(message)
    }

    pub fn delete_message(&self, message: DeleteMessagePy) -> bool {
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(delete_message(&self.client, &message.msg))
        })
    }

    /// 仅作占位
    /// (因为目前来说, rust调用 Python端没法启动一个异步运行时
    /// 所以只能 tokio::task::block_in_place 转换成同步调用)
    // #[staticmethod]
    // pub fn send_message_a(
    //     py: Python,
    //     client: IcaClientPy,
    //     message: SendMessagePy,
    // ) -> PyResult<&PyAny> {
    //     pyo3_asyncio::tokio::future_into_py(py, async move {
    //         Ok(send_message(&client.client, &message.msg).await)
    //     })
    // }

    #[getter]
    pub fn get_status(&self) -> IcaStatusPy { IcaStatusPy::new() }
    #[getter]
    pub fn get_version(&self) -> String { crate::VERSION.to_string() }
    #[getter]
    pub fn get_version_str(&self) -> String { crate::version_str() }
    #[getter]
    pub fn get_client_id(&self) -> String { crate::client_id() }
    #[getter]
    pub fn get_ica_version(&self) -> String { crate::ICA_VERSION.to_string() }
    #[getter]
    pub fn get_startup_time(&self) -> SystemTime { crate::start_up_time() }

    #[getter]
    pub fn get_py_tasks_count(&self) -> usize {
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(async { crate::py::call::PY_TASKS.lock().await.len_check() })
        })
    }

    /// 重新加载插件状态
    /// 返回是否成功
    pub fn reload_plugin_status(&self) -> bool { PyStatus::get_mut().config.reload_from_default() }

    /// 设置某个插件的状态
    pub fn set_plugin_status(&self, plugin_name: String, status: bool) {
        PyStatus::get_mut().set_status(&plugin_name, status);
    }

    pub fn get_plugin_status(&self, plugin_name: String) -> Option<bool> {
        PyStatus::get().get_status(&plugin_name)
    }

    /// 同步状态到配置文件
    /// 这样关闭的时候就会保存状态
    pub fn sync_status_to_config(&self) { PyStatus::get_mut().config.sync_status_to_config(); }

    /// 重新加载插件
    ///
    /// 返回是否成功
    pub fn reload_plugin(&self, plugin_name: String) -> bool {
        PyStatus::get_mut().reload_plugin(&plugin_name)
    }

    pub fn debug(&self, content: String) {
        event!(Level::DEBUG, "{}", content);
    }
    pub fn info(&self, content: String) {
        event!(Level::INFO, "{}", content);
    }
    pub fn warn(&self, content: String) {
        event!(Level::WARN, "{}", content);
    }
}

impl IcaClientPy {
    pub fn new(client: &Client) -> Self {
        Self {
            client: client.clone(),
        }
    }
}
