use std::time::SystemTime;

use pyo3::prelude::*;

use rust_socketio::asynchronous::Client;
use tokio::runtime::Runtime;
use tracing::{debug, info, warn};

use crate::data_struct::tailchat::messages::{ReceiveMessage, SendingFile, SendingMessage};
use crate::data_struct::tailchat::{ConverseId, GroupId, MessageId, UserId};
use crate::tailchat::client::send_message;

#[pyclass]
#[pyo3(name = "TailchatClient")]
pub struct TailchatClientPy {
    pub client: Client,
}

impl TailchatClientPy {
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
    pub fn from_recive_message(msg: &ReceiveMessage) -> Self {
        Self {
            message: msg.clone(),
        }
    }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "TailchatSendingMessage")]
pub struct TailchatSendingMessagePy {
    pub message: SendingMessage,
}

#[pymethods]
impl TailchatClientPy {
    pub fn send_message(&self, message: TailchatSendingMessagePy) -> bool {
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(send_message(&self.client, &message.message))
        })
    }

    pub fn send_and_warn(&self, message: TailchatSendingMessagePy) -> bool {
        warn!("{}", message.message.content);
        self.send_message(message)
    }
    #[getter]
    pub fn get_version(&self) -> String { crate::VERSION.to_string() }
    #[getter]
    pub fn get_version_str(&self) -> String { crate::version_str() }
    #[getter]
    pub fn get_client_id(&self) -> String { crate::client_id() }
    #[getter]
    pub fn get_tailchat_version(&self) -> String { crate::TAILCHAT_VERSION.to_string() }
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

    #[pyo3(signature = (content, converse_id, group_id = None))]
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
    pub fn debug(&self, content: String) {
        debug!("{}", content);
    }
    pub fn info(&self, content: String) {
        info!("{}", content);
    }
    pub fn warn(&self, content: String) {
        warn!("{}", content);
    }
}

#[pymethods]
impl TailchatReceiveMessagePy {
    #[getter]
    pub fn get_is_reply(&self) -> bool { self.message.is_reply() }
    #[getter]
    pub fn get_is_from_self(&self) -> bool { self.message.is_from_self() }
    #[getter]
    pub fn get_msg_id(&self) -> MessageId { self.message.msg_id.clone() }
    #[getter]
    pub fn get_content(&self) -> String { self.message.content.clone() }
    #[getter]
    pub fn get_sender_id(&self) -> UserId { self.message.sender_id.clone() }
    #[getter]
    pub fn get_group_id(&self) -> Option<GroupId> { self.message.group_id.clone() }
    #[getter]
    pub fn get_converse_id(&self) -> ConverseId { self.message.converse_id.clone() }
    /// 作为回复
    pub fn as_reply(&self) -> TailchatSendingMessagePy {
        TailchatSendingMessagePy {
            message: self.message.as_reply(),
        }
    }
    pub fn reply_with(&self, content: String) -> TailchatSendingMessagePy {
        TailchatSendingMessagePy {
            message: self.message.reply_with(&content),
        }
    }
}

#[pymethods]
impl TailchatSendingMessagePy {
    #[getter]
    pub fn get_content(&self) -> String { self.message.content.clone() }
    #[setter]
    pub fn set_content(&mut self, content: String) { self.message.content = content; }
    #[getter]
    pub fn get_converse_id(&self) -> ConverseId { self.message.converse_id.clone() }
    #[setter]
    pub fn set_converse_id(&mut self, converse_id: ConverseId) {
        self.message.converse_id = converse_id;
    }
    #[getter]
    pub fn get_group_id(&self) -> Option<GroupId> { self.message.group_id.clone() }
    #[setter]
    pub fn set_group_id(&mut self, group_id: Option<GroupId>) { self.message.group_id = group_id; }
    pub fn with_content(&mut self, content: String) -> Self {
        self.message.content = content;
        self.clone()
    }
    pub fn clear_meta(&mut self) -> Self {
        self.message.meta = None;
        self.clone()
    }
    pub fn set_img(&mut self, file: Vec<u8>, file_name: String) {
        let file = SendingFile::Image {
            file,
            name: file_name,
        };
        self.message.add_img(file);
    }
}
