//! 暴露给 Python 插件的 Tailchat 消息和客户端类型。

use std::time::SystemTime;

use pyo3::prelude::*;

use rust_socketio::asynchronous::Client;
use tokio::runtime::Runtime;
use tracing::{debug, info, warn};

use crate::data_struct::tailchat::messages::{ReceiveMessage, SendingFile, SendingMessage};
use crate::data_struct::tailchat::{ConverseId, GroupId, MessageId, UserId};
use crate::py::PY_PLUGIN_STORAGE;
use crate::tailchat::client::send_message;

#[pyclass]
#[pyo3(name = "TailchatClient")]
pub struct TailchatClientPy {
    pub client: Client,
}

impl TailchatClientPy {
    /// 创建并初始化对应的数据结构。
    pub fn new(client: &Client) -> Self {
        Self {
            client: client.clone(),
        }
    }
}

#[pyclass]
#[pyo3(name = "TailchatStatus")]
/// 预留?
pub struct TailchatStatusPy {}

#[pyclass]
#[pyo3(name = "TailchatReceiveMessage")]
pub struct TailchatReceiveMessagePy {
    pub message: ReceiveMessage,
}

impl TailchatReceiveMessagePy {
    /// 从 `recive_message` 构造当前值。
    pub fn from_recive_message(msg: &ReceiveMessage) -> Self {
        Self {
            message: msg.clone(),
        }
    }
}

#[derive(Clone)]
#[pyclass(from_py_object)]
#[pyo3(name = "TailchatSendingMessage")]
pub struct TailchatSendingMessagePy {
    pub message: SendingMessage,
}

#[pymethods]
impl TailchatClientPy {
    /// 发送 `message` 请求或消息。
    pub fn send_message(&self, message: TailchatSendingMessagePy) -> bool {
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(send_message(&self.client, &message.message))
        })
    }

    /// 发送 `and_warn` 请求或消息。
    pub fn send_and_warn(&self, message: TailchatSendingMessagePy) -> bool {
        warn!("{}", message.message.content);
        self.send_message(message)
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
    /// 返回 `tailchat_version` 对应的数据。
    pub fn get_tailchat_version(&self) -> String { crate::TAILCHAT_VERSION.to_string() }
    #[getter]
    /// 返回 `startup_time` 对应的数据。
    pub fn get_startup_time(&self) -> SystemTime { crate::start_up_time() }

    #[getter]
    /// 返回 `py_tasks_count` 对应的数据。
    pub fn get_py_tasks_count(&self) -> usize {
        tokio::task::block_in_place(|| {
            let rt = Runtime::new().unwrap();
            rt.block_on(async { crate::py::call::PY_TASKS.lock().await.total_len() })
        })
    }

    /// 重新加载插件状态
    /// 返回是否成功
    pub fn sync_status_from_file(&self) {
        let mut storage = PY_PLUGIN_STORAGE.blocking_lock();
        storage.sync_status_from_file();
    }

    /// 同步状态到配置文件
    /// 这样关闭的时候就会保存状态
    pub fn sync_status_to_file(&self) {
        let storage = PY_PLUGIN_STORAGE.blocking_lock();
        storage.sync_status_to_file();
    }

    /// 设置某个插件的状态
    pub fn set_plugin_status(&self, plugin_name: String, status: bool) {
        let mut storage = PY_PLUGIN_STORAGE.blocking_lock();
        let _ = storage.set_status(&plugin_name, status);
    }

    /// 返回 `plugin_status` 对应的数据。
    pub fn get_plugin_status(&self, plugin_name: String) -> Option<bool> {
        let storage = PY_PLUGIN_STORAGE.blocking_lock();
        storage.get_status(&plugin_name)
    }

    /// 重新加载插件
    ///
    /// 返回是否成功
    pub fn reload_plugin(&self, plugin_name: String) -> bool {
        let mut storage = PY_PLUGIN_STORAGE.blocking_lock();
        storage
            .storage
            .get_mut(&plugin_name)
            .map(|p| p.reload_self(None).is_ok())
            .unwrap_or(false)
    }

    #[pyo3(signature = (content, converse_id, group_id = None))]
    /// 创建并初始化对应的数据结构。
    pub fn new_message(
        &self,
        content: String,
        converse_id: ConverseId,
        group_id: Option<GroupId>,
    ) -> TailchatSendingMessagePy {
        TailchatSendingMessagePy {
            message: SendingMessage::new(content, converse_id, group_id, None),
        }
    }
    /// 向 Python 插件日志记录调试信息。
    pub fn debug(&self, content: String) {
        debug!("{}", content);
    }
    /// 向 Python 插件日志记录普通信息。
    pub fn info(&self, content: String) {
        info!("{}", content);
    }
    /// 向 Python 插件日志记录警告信息。
    pub fn warn(&self, content: String) {
        warn!("{}", content);
    }
}

#[pymethods]
impl TailchatReceiveMessagePy {
    #[getter]
    /// 返回 `is_reply` 对应的数据。
    pub fn get_is_reply(&self) -> bool { self.message.is_reply() }
    #[getter]
    /// 返回 `is_from_self` 对应的数据。
    pub fn get_is_from_self(&self) -> bool { self.message.is_from_self() }
    #[getter]
    /// 返回 `msg_id` 对应的数据。
    pub fn get_msg_id(&self) -> MessageId { self.message.msg_id.clone() }
    #[getter]
    /// 返回 `content` 对应的数据。
    pub fn get_content(&self) -> String { self.message.content.clone() }
    #[getter]
    /// 返回 `sender_id` 对应的数据。
    pub fn get_sender_id(&self) -> UserId { self.message.sender_id.clone() }
    #[getter]
    /// 返回 `group_id` 对应的数据。
    pub fn get_group_id(&self) -> Option<GroupId> { self.message.group_id.clone() }
    #[getter]
    /// 返回 `converse_id` 对应的数据。
    pub fn get_converse_id(&self) -> ConverseId { self.message.converse_id.clone() }
    /// 作为回复
    pub fn as_reply(&self) -> TailchatSendingMessagePy {
        TailchatSendingMessagePy {
            message: self.message.as_reply(),
        }
    }
    /// 构造回复当前消息的新消息。
    pub fn reply_with(&self, content: String) -> TailchatSendingMessagePy {
        TailchatSendingMessagePy {
            message: self.message.reply_with(&content),
        }
    }
}

#[pymethods]
impl TailchatSendingMessagePy {
    #[getter]
    /// 返回 `content` 对应的数据。
    pub fn get_content(&self) -> String { self.message.content.clone() }
    #[setter]
    /// 更新 `content` 对应的数据。
    pub fn set_content(&mut self, content: String) { self.message.content = content; }
    #[getter]
    /// 返回 `converse_id` 对应的数据。
    pub fn get_converse_id(&self) -> ConverseId { self.message.converse_id.clone() }
    #[setter]
    /// 更新 `converse_id` 对应的数据。
    pub fn set_converse_id(&mut self, converse_id: ConverseId) {
        self.message.converse_id = converse_id;
    }
    #[getter]
    /// 返回 `group_id` 对应的数据。
    pub fn get_group_id(&self) -> Option<GroupId> { self.message.group_id.clone() }
    #[setter]
    /// 更新 `group_id` 对应的数据。
    pub fn set_group_id(&mut self, group_id: Option<GroupId>) { self.message.group_id = group_id; }
    /// 设置消息内容并返回更新后的值。
    pub fn with_content(&mut self, content: String) -> Self {
        self.message.content = content;
        self.clone()
    }
    /// 清除发送消息携带的元数据。
    pub fn clear_meta(&mut self) -> Self {
        self.message.meta = None;
        self.clone()
    }
    /// 更新 `img` 对应的数据。
    pub fn set_img(&mut self, file: Vec<u8>, file_name: String) {
        let file = SendingFile::Image {
            file,
            name: file_name,
        };
        self.message.add_img(file);
    }
}
