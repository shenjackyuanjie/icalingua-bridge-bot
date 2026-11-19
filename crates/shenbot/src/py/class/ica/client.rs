//! Python 插件使用的 ICA 客户端和宿主状态视图。

use std::{sync::Arc, time::SystemTime};

use pyo3::{
    Bound, Py, PyAny, PyResult, Python,
    exceptions::{PyRuntimeError, PyValueError},
    pyclass, pymethods,
};
use rust_socketio::asynchronous::Client;
use serde_json::{Value, json};
use tracing::{Level, event};

use crate::data_struct::ica::messages::raw::RawSendMessage;
use crate::data_struct::ica::messages::{DeleteMessage, Message, NewMessage, SendMessage};
use crate::data_struct::ica::{MessageId, RoomId, UserId};
use crate::ica::client::{bridge, files};
use crate::ica::client::{
    delete_message, get_group_members, get_muted_group_members, get_nt_pic_url_by_file_id,
    handle_request, report_read, send_message, send_poke, send_room_sign_in, send_string_message,
    set_group_ban,
};
use crate::py::{
    binding::RuntimeBinding,
    host::PluginHost,
    json::{json_to_py, py_to_json},
    storage,
};

use super::{
    bridge_contract::{decode_query_messages, json_args, query_timeout, validate_forward_nodes},
    message::{DeleteMessagePy, NewMessagePy},
    room::{IcaGroupMemberPy, IcaRoomPy},
    send::SendMessagePy,
};

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
#[pyo3(name = "IcaClient")]
pub struct IcaClientPy {
    pub client: Client,
    binding: RuntimeBinding,
}

#[pymethods]
#[allow(clippy::too_many_arguments)]
impl IcaClientPy {
    /// 复用现有连接发送事件；返回 true 仅表示 Socket.IO 受理，不表示 QQ 执行成功。
    #[pyo3(signature = (event, args = None))]
    pub fn emit_event(
        &self,
        py: Python<'_>,
        event: String,
        args: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let args = json_args(args)?;
        self.binding.with_host(py, |host| {
            host.context
                .runtime
                .block_on(bridge::emit_event(&self.client, &event, args))
                .map(|_| true)
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// 请求真实 ACK；有独立超时，返回完整业务值，保留数组/对象的原始形状。
    /// 收藏/闪传事件的 args 为单个带 action 的对象组成的列表；业务拒绝抛出 RuntimeError。
    #[pyo3(signature = (event, args = None, timeout = 15.0))]
    pub fn request_event(
        &self,
        py: Python<'_>,
        event: String,
        args: Option<&Bound<'_, PyAny>>,
        timeout: f64,
    ) -> PyResult<Py<PyAny>> {
        let args = json_args(args)?;
        let timeout = query_timeout(timeout)?;
        let value = self
            .binding
            .with_host(py, |host| {
                host.context.runtime.block_on(bridge::request_event(
                    &self.client,
                    &event,
                    args,
                    timeout,
                ))
            })?
            .map_err(PyRuntimeError::new_err)?;
        json_to_py(py, &value)
    }

    /// 从 Bridge 存储读取一页历史；options 为 before/after 游标，不自动拉 QQ 全历史。
    #[pyo3(signature = (room_id, options = None, timeout = 15.0))]
    pub fn fetch_messages(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        options: Option<&Bound<'_, PyAny>>,
        timeout: f64,
    ) -> PyResult<Vec<NewMessagePy>> {
        let options = options.map(py_to_json).transpose()?.unwrap_or_else(|| json!({}));
        let timeout = query_timeout(timeout)?;
        self.query_messages(py, room_id, true, |host| {
            host.context.runtime.block_on(bridge::fetch_messages(
                &self.client,
                room_id,
                options,
                timeout,
            ))
        })
    }

    /// room_id=0 时跨房间搜索；结果自带 roomId，缓存按实际来源房间隔离。
    #[pyo3(signature = (room_id, keyword, offset = 0, sender_id = None, start_time = None, end_time = None, timeout = 15.0))]
    pub fn search_messages(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        keyword: String,
        offset: usize,
        sender_id: Option<UserId>,
        start_time: Option<i64>,
        end_time: Option<i64>,
        timeout: f64,
    ) -> PyResult<Vec<NewMessagePy>> {
        let timeout = query_timeout(timeout)?;
        self.query_messages(py, room_id, true, |host| {
            host.context.runtime.block_on(bridge::search_messages(
                &self.client,
                room_id,
                &keyword,
                offset,
                sender_id,
                start_time,
                end_time,
                timeout,
            ))
        })
    }

    #[pyo3(signature = (room_id, sender_id, offset = 0, snapshot_time = None, timeout = 15.0))]
    pub fn fetch_messages_by_sender(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        sender_id: UserId,
        offset: usize,
        snapshot_time: Option<i64>,
        timeout: f64,
    ) -> PyResult<Vec<NewMessagePy>> {
        let timeout = query_timeout(timeout)?;
        self.query_messages(py, room_id, true, |host| {
            host.context.runtime.block_on(bridge::fetch_messages_by_sender(
                &self.client,
                room_id,
                sender_id,
                offset,
                snapshot_time,
                timeout,
            ))
        })
    }

    #[pyo3(signature = (room_id, message_id, before = 20, after = 20, timeout = 15.0))]
    pub fn fetch_messages_around(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        message_id: String,
        before: usize,
        after: usize,
        timeout: f64,
    ) -> PyResult<Vec<NewMessagePy>> {
        let timeout = query_timeout(timeout)?;
        self.query_messages(py, room_id, true, |host| {
            host.context.runtime.block_on(bridge::fetch_messages_around(
                &self.client,
                room_id,
                &message_id,
                before,
                after,
                timeout,
            ))
        })
    }

    #[pyo3(signature = (room_id, offset = 0, end_time = None, timeout = 15.0))]
    pub fn fetch_image_messages(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        offset: usize,
        end_time: Option<i64>,
        timeout: f64,
    ) -> PyResult<Vec<NewMessagePy>> {
        let timeout = query_timeout(timeout)?;
        self.query_messages(py, room_id, true, |host| {
            host.context.runtime.block_on(bridge::fetch_image_messages(
                &self.client,
                room_id,
                offset,
                end_time,
                timeout,
            ))
        })
    }

    /// 查询有界会话缓存，不触网、不重新触发新消息 hook。
    #[pyo3(signature = (room_id, limit = 200))]
    pub fn get_loaded_messages(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        limit: usize,
    ) -> PyResult<Vec<NewMessagePy>> {
        let messages = self.binding.with_host(py, |host| {
            host.context.read_ica(|state| state.get_loaded_messages(room_id, limit))
        })?;
        Ok(messages
            .into_iter()
            .map(|message| {
                NewMessagePy::new(&NewMessage::new(room_id, message), &self.binding.context)
            })
            .collect())
    }

    #[pyo3(signature = (timeout = 15.0))]
    pub fn get_system_messages(&self, py: Python<'_>, timeout: f64) -> PyResult<Py<PyAny>> {
        let timeout = query_timeout(timeout)?;
        let value = self
            .binding
            .with_host(py, |host| {
                host.context
                    .runtime
                    .block_on(bridge::get_system_messages(&self.client, timeout))
            })?
            .map_err(PyRuntimeError::new_err)?;
        json_to_py(py, &value)
    }

    #[pyo3(signature = (timeout = 15.0))]
    pub fn get_friends(&self, py: Python<'_>, timeout: f64) -> PyResult<Py<PyAny>> {
        let timeout = query_timeout(timeout)?;
        let values = self
            .binding
            .with_host(py, |host| {
                host.context.runtime.block_on(bridge::get_friends(&self.client, timeout))
            })?
            .map_err(PyRuntimeError::new_err)?;
        json_to_py(py, &Value::Array(values))
    }

    #[pyo3(signature = (timeout = 15.0))]
    pub fn get_groups(&self, py: Python<'_>, timeout: f64) -> PyResult<Py<PyAny>> {
        let timeout = query_timeout(timeout)?;
        let values = self
            .binding
            .with_host(py, |host| {
                host.context.runtime.block_on(bridge::get_groups(&self.client, timeout))
            })?
            .map_err(PyRuntimeError::new_err)?;
        json_to_py(py, &Value::Array(values))
    }

    /// 展开合并转发资源，room_id 为 0；不会把资源内容混进真实会话缓存。
    #[pyo3(signature = (res_id, file_name = None, timeout = 15.0))]
    pub fn get_forward_messages(
        &self,
        py: Python<'_>,
        res_id: String,
        file_name: Option<String>,
        timeout: f64,
    ) -> PyResult<Vec<NewMessagePy>> {
        let timeout = query_timeout(timeout)?;
        self.query_messages(py, 0, false, |host| {
            host.context.runtime.block_on(bridge::get_forward_messages(
                &self.client,
                &res_id,
                file_name.as_deref(),
                timeout,
            ))
        })
    }

    /// 向目标房间发送 makeForward；nodes 使用本体 fakes 节点结构，不伪造发送 ACK。
    #[pyo3(signature = (nodes, target, dm = None, origin = None))]
    pub fn send_merged_forward(
        &self,
        py: Python<'_>,
        nodes: &Bound<'_, PyAny>,
        target: RoomId,
        dm: Option<bool>,
        origin: Option<i64>,
    ) -> PyResult<bool> {
        let nodes = py_to_json(nodes)?;
        validate_forward_nodes(&nodes, target)?;
        let args = vec![nodes, json!(dm.unwrap_or(target > 0)), json!(origin), json!(target)];
        self.binding.with_host(py, |host| {
            host.context
                .runtime
                .block_on(bridge::emit_event(&self.client, "makeForward", args))
                .map(|_| true)
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// 分片上传文件，返回 Bridge 哈希引用；不提交到 QQ。
    pub fn upload_file(
        &self,
        py: Python<'_>,
        file_name: String,
        data: Vec<u8>,
    ) -> PyResult<String> {
        self.binding.with_host(py, |host| {
            host.context
                .runtime
                .block_on(files::upload_file_to_bridge(&self.client, &file_name, &data))
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// 上传并发送普通文件；图片/语音请用消息媒体 API。返回 true 仅表示发送受理。
    #[pyo3(signature = (room_id, file_name, data, content = "".to_string(), mime = "application/octet-stream".to_string(), reply = None, at = None))]
    pub fn send_file(
        &self,
        py: Python<'_>,
        room_id: RoomId,
        file_name: String,
        data: Vec<u8>,
        content: String,
        mime: String,
        reply: Option<&Bound<'_, PyAny>>,
        at: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let reply = reply.map(py_to_json).transpose()?;
        let at = json_args(at)?;
        self.binding.with_host(py, |host| {
            host.context
                .runtime
                .block_on(files::upload_and_send_file(
                    &self.client,
                    room_id,
                    &content,
                    reply,
                    at,
                    &file_name,
                    &mime,
                    &data,
                ))
                .map(|_| true)
                .map_err(PyRuntimeError::new_err)
        })?
    }

    /// group_id 使用正群号；只有 uploadGroupFile 返回真实业务 {ok,error?} ACK。
    #[pyo3(signature = (group_id, file_name, data, parent = "/".to_string()))]
    pub fn upload_group_file(
        &self,
        py: Python<'_>,
        group_id: i64,
        file_name: String,
        data: Vec<u8>,
        parent: String,
    ) -> PyResult<Py<PyAny>> {
        let value = self
            .binding
            .with_host(py, |host| {
                host.context.runtime.block_on(files::upload_group_file(
                    &self.client,
                    group_id,
                    &parent,
                    &file_name,
                    &data,
                ))
            })?
            .map_err(PyRuntimeError::new_err)?;
        json_to_py(py, &value)
    }

    /// 使用 GFS token 建独立 fileMgr 角色连接，完成请求后断开。
    #[pyo3(signature = (group_id, event, args = None, timeout = 15.0))]
    pub fn file_manager_request(
        &self,
        py: Python<'_>,
        group_id: i64,
        event: String,
        args: Option<&Bound<'_, PyAny>>,
        timeout: f64,
    ) -> PyResult<Py<PyAny>> {
        let args = json_args(args)?;
        let timeout = query_timeout(timeout)?;
        let value = self
            .binding
            .with_host(py, |host| {
                host.context.runtime.block_on(files::file_manager_request(
                    &self.client,
                    &host.context,
                    group_id,
                    &event,
                    args,
                    timeout,
                ))
            })?
            .map_err(PyRuntimeError::new_err)?;
        json_to_py(py, &value)
    }

    /// 构造新消息；reply 可直接使用查询结果的 to_dict()。
    #[pyo3(signature = (room_id, content = "".to_string(), reply = None))]
    pub fn new_message(
        &self,
        room_id: RoomId,
        content: String,
        reply: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<SendMessagePy> {
        if room_id == 0 {
            return Err(PyValueError::new_err("发送房间不能为 0"));
        }
        let reply = reply
            .map(py_to_json)
            .transpose()?
            .map(|value| Message::new_from_json(&value).map(|message| message.as_reply()))
            .transpose()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(SendMessagePy::new(SendMessage::new(content, room_id, reply)))
    }

    /// 发送 QQ 已读上报事件；只保证 emit，失败时抛 RuntimeError。
    /// 当前独立 oicq Bridge 未注册 reportRead，emit 成功不表示 QQ 已读已上报。
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

    /// 发送 oicq 原始消息节点链；非法 JSON/节点抛 ValueError，不发送空消息。
    /// 返回值仅表示传输受理，不是 QQ 发送成功或消息 ID。
    pub fn send_raw_message(
        &self,
        py: Python<'_>,
        raw_msg: String,
        room_id: RoomId,
    ) -> PyResult<bool> {
        let msg =
            RawSendMessage::string_to_json(&raw_msg, room_id).map_err(PyValueError::new_err)?;
        self.binding.with_host(py, |host| {
            host.context
                .runtime
                .block_on(send_string_message(&self.client, &msg, &host.context))
        })
    }

    /// 发送 `and_warn` 请求或消息。
    pub fn send_and_warn(&self, py: Python<'_>, message: SendMessagePy) -> bool {
        event!(target: super::LOG_TARGET, Level::WARN, message.msg.content);
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
    pub fn get_status(&self) -> super::IcaStatusPy {
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
        event!(target: super::LOG_TARGET, Level::DEBUG, "{}", content);
    }
    /// 向 Python 插件日志记录普通信息。
    pub fn info(&self, content: String) {
        event!(target: super::LOG_TARGET, Level::INFO, "{}", content);
    }
    /// 向 Python 插件日志记录警告信息。
    pub fn warn(&self, content: String) {
        event!(target: super::LOG_TARGET, Level::WARN, "{}", content);
    }
}

impl IcaClientPy {
    /// 在宿主的阻塞任务中查询，结果只入有界缓存，不重放自动回复 hook。
    fn query_messages(
        &self,
        py: Python<'_>,
        fallback_room: RoomId,
        cache: bool,
        query: impl FnOnce(&Arc<PluginHost>) -> Result<Vec<Value>, String> + Send,
    ) -> PyResult<Vec<NewMessagePy>> {
        let messages = self
            .binding
            .with_host(py, |host| {
                let messages = query(host)?;
                let messages = decode_query_messages(messages, fallback_room, cache)?;
                if cache {
                    host.context.update_ica(|state| {
                        let mut rooms = std::collections::HashMap::<RoomId, Vec<Message>>::new();
                        for message in &messages {
                            if message.room_id != 0 {
                                rooms.entry(message.room_id).or_default().push(message.msg.clone());
                            }
                        }
                        for (room, messages) in rooms {
                            state.store_loaded_messages(room, messages, false);
                        }
                    });
                }
                Ok::<_, String>(messages)
            })?
            .map_err(PyRuntimeError::new_err)?;
        Ok(messages
            .iter()
            .map(|message| NewMessagePy::new(message, &self.binding.context))
            .collect())
    }

    /// 创建并初始化对应的数据结构。
    pub fn new(client: &Client, host: &Arc<PluginHost>) -> Self {
        Self {
            client: client.clone(),
            binding: RuntimeBinding::new(host),
        }
    }
}
