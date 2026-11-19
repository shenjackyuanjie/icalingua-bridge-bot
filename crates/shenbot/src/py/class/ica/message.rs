//! 收到的 ICA 消息及其回复和撤回封装。

use std::sync::Arc;

use pyo3::{Py, PyAny, PyResult, Python, pyclass, pymethods};
use serde_json::{Value, json};

use crate::data_struct::ica::messages::{DeleteMessage, MessageTrait, NewMessage, ReplyMessage};
use crate::data_struct::ica::{MessageId, RoomId, RoomIdTrait};
use crate::py::json::json_to_py;

use super::{bridge_contract::forward_node, send::SendMessagePy};

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
    /// 当前消息的结构化媒体快照，含 fid、顺序、尺寸及未知媒体字段。
    #[getter]
    pub fn get_files(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &json!(self.msg.msg.files))
    }
    #[getter]
    pub fn get_file(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &json!(self.msg.msg.file))
    }
    #[getter]
    pub fn get_at(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &json!(self.msg.msg.at))
    }
    #[getter]
    pub fn get_code(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &self.msg.msg.code)
    }
    /// Bridge 毫秒时间；缺省时间保持 None，不为插件伪造服务器时间。
    #[getter]
    pub fn get_time(&self) -> Option<i64> {
        self.msg
            .msg
            .raw_data
            .get("time")
            .and_then(Value::as_i64)
            .and_then(crate::data_struct::ica::messages::parse_bridge_timestamp)
            .map(|time| time.timestamp_millis())
    }
    #[getter]
    pub fn get_raw_data(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &self.msg.msg.raw_data)
    }
    #[getter]
    pub fn get_raw_msg(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &self.msg.msg.raw_msg)
    }
    #[getter]
    pub fn get_reply(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &json!(self.msg.msg.reply))
    }
    #[getter]
    pub fn get_deleted(&self) -> bool { self.msg.msg.deleted }
    #[getter]
    pub fn get_hide(&self) -> bool { self.msg.msg.hide }
    #[getter]
    pub fn get_reveal(&self) -> bool { self.msg.msg.reveal }
    #[getter]
    pub fn get_markdown(&self) -> bool { self.msg.msg.markdown }
    #[getter]
    pub fn get_button_rows(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &self.msg.msg.button_rows)
    }
    #[getter]
    pub fn get_recall_info(&self) -> Option<String> { self.msg.msg.recall_info.clone() }
    /// 输出规范化 Bridge Message 并附带实际 roomId；不会返回缓存的可变引用。
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let mut value = self.msg.msg.to_bridge_value();
        value["roomId"] = json!(self.msg.room_id);
        json_to_py(py, &value)
    }
    /// 重建 oicq FakeMessage 节点；普通文件等不可合并资源会显式降级为文本。
    pub fn to_forward_node(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &forward_node(&self.msg.msg))
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
