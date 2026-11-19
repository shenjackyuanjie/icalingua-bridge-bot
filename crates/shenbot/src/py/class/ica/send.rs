//! 供 Python 插件构造发送消息的封装。

use pyo3::{Bound, Py, PyAny, PyResult, Python, exceptions::PyRuntimeError, pyclass, pymethods};

use crate::data_struct::ica::{RoomId, messages::SendMessage};
use crate::py::json::{json_to_py, py_to_json};

use super::bridge_contract::validate_mentions;

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
    /// 发送时的 @ 数组，与接收消息的 boolean/'all' 不同。
    #[getter]
    pub fn get_at(&self, py: Python<'_>) -> PyResult<Py<PyAny>> { json_to_py(py, &self.msg.at) }
    #[setter]
    pub fn set_at(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let at = py_to_json(value)?;
        validate_mentions(&at)?;
        self.msg.at = at;
        Ok(())
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
