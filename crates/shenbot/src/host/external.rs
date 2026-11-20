//! 外部插件动作与既有 ICA 连接的适配；不持有 BotHost，避免形成引用环。

use std::{
    sync::{Arc, Mutex, OnceLock, RwLock},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ica_ext_sdk::{
    ConnectionState, HelloAck, HostToPlugin, Image, MessageFile, NewMessage, ReplyMessage, Room,
    SendMessage, async_trait,
};
use rust_socketio::asynchronous::Client;
use tokio::io::AsyncReadExt;

use crate::{
    config::{ExtConfig, ExtPluginConfig},
    context::BotContext,
    data_struct::ica::{all_rooms, files, messages},
    ext::{ExtHostServices, ExtPluginHost, ExtPluginState},
    ica::{client, connection::Connection},
};

use super::{
    PluginAction, PluginChange,
    plugin::{PluginKind, PluginSummary},
};

pub(super) struct ExternalBackend {
    core: Arc<ExtPluginHost>,
    services: Arc<IcaServices>,
    watcher: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ExternalBackend {
    pub(super) fn new(
        context: &Arc<BotContext>,
        config: ExtConfig,
        units: Vec<ExtPluginConfig>,
    ) -> anyhow::Result<Self> {
        let services = Arc::new(IcaServices::new(context.clone()));
        let core = Arc::new(ExtPluginHost::new(config, units, services.clone())?);
        Ok(Self {
            core,
            services,
            watcher: Mutex::default(),
        })
    }

    pub(super) async fn start(&self) {
        self.core.start_enabled().await;
        let mut watcher = self.watcher.lock().expect("外部插件连接观察器锁被污染");
        if watcher.is_none() {
            *watcher = Some(self.services.watch(&self.core));
        }
    }

    pub(super) fn stop_accepting(&self) { self.core.stop_accepting(); }

    pub(super) async fn shutdown(&self) {
        let watcher = self.watcher.lock().expect("外部插件连接观察器锁被污染").take();
        if let Some(watcher) = watcher {
            watcher.abort();
            let _ = watcher.await;
        }
        self.core.shutdown().await;
    }

    pub(super) fn summaries(&self) -> impl Iterator<Item = PluginSummary> {
        self.core.summaries().into_iter().map(|entry| PluginSummary {
            name: entry.name.unwrap_or_else(|| entry.id.clone()),
            id: entry.id,
            kind: PluginKind::Ext,
            enabled: entry.enabled,
            state: Some(match entry.state {
                ExtPluginState::Stopped => "停止",
                ExtPluginState::Starting => "启动中",
                ExtPluginState::Active => "活动",
                ExtPluginState::Draining => "排空中",
                ExtPluginState::Backoff => "等待重启",
                ExtPluginState::Failed => "失败",
            }),
        })
    }

    pub(super) async fn change(&self, id: &str, action: PluginAction) -> PluginChange {
        let Some(plugin) = self.core.summaries().into_iter().find(|entry| entry.id == id) else {
            return PluginChange::NotFound;
        };
        let result = match (action, plugin.enabled) {
            (PluginAction::Enable, true) | (PluginAction::Disable, false) => {
                return PluginChange::Unchanged;
            }
            (PluginAction::Enable, false) => self.core.enable(id).await,
            (PluginAction::Disable, true) => self.core.disable(id).await,
            (PluginAction::Reload, _) => self.core.reload(id).await,
        };
        match result {
            Ok(()) => PluginChange::Changed,
            Err(error) => PluginChange::Failed(error.into()),
        }
    }

    pub(super) fn dispatch_message(&self, generation: u64, message: &messages::NewMessage) {
        if let Some(snapshot) = self.services.message(generation, message) {
            if message.msg.system {
                self.core.dispatch_system_message(snapshot);
            } else {
                self.core.dispatch(HostToPlugin::NewMessage(snapshot));
            }
        }
    }

    pub(super) fn dispatch_rooms(&self, generation: u64, rooms: &[all_rooms::Room], full: bool) {
        if let Some(rooms) = self.services.update_rooms(generation, rooms, full) {
            self.core.dispatch(HostToPlugin::RoomsUpdated { rooms });
        }
    }
}

impl Drop for ExternalBackend {
    fn drop(&mut self) {
        if let Some(watcher) =
            self.watcher.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).take()
        {
            watcher.abort();
        }
    }
}

// NDJSON 只传路径；路径与内联附件共享一个独立的解码后总量预算。
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

struct IcaServices {
    context: Arc<BotContext>,
    expected_account: OnceLock<i64>,
    // 不能在重连或换 QQ 后把上一连接的群名装入 hello_ack。
    rooms: RwLock<(u64, Vec<Room>)>,
}

impl IcaServices {
    fn new(context: Arc<BotContext>) -> Self {
        let account = OnceLock::new();
        if let Some(id) =
            context.config.ica.as_ref().map(|config| config.self_id).filter(|id| *id > 0)
        {
            let _ = account.set(id);
        }
        Self {
            context,
            expected_account: account,
            rooms: RwLock::default(),
        }
    }

    fn connection(&self) -> &Connection<Client> { &self.context.ica_connection }

    /// 复用不轮询的连接观察器。断线时推送离线及空房间，重连只发布新代快照。
    pub(super) fn watch(self: &Arc<Self>, ext: &Arc<ExtPluginHost>) -> tokio::task::JoinHandle<()> {
        let mut updates = self.connection().subscribe();
        let services = Arc::downgrade(self);
        let ext = Arc::downgrade(ext);
        tokio::spawn(async move {
            let mut last = None;
            loop {
                let current = updates.borrow_and_update().clone();
                {
                    let (Some(services), Some(ext)) = (services.upgrade(), ext.upgrade()) else {
                        break;
                    };
                    let state = services.state(&current);
                    let comparable = (current.generation, state.ready, state.online, state.account);
                    if last != Some(comparable) {
                        let rooms_changed =
                            last.map_or(state.ready, |(generation, ready, _, _)| {
                                generation != current.generation || ready != state.ready
                            });
                        ext.dispatch(HostToPlugin::ConnectionState(state));
                        if rooms_changed {
                            ext.dispatch(HostToPlugin::RoomsUpdated {
                                rooms: services.rooms(&current),
                            });
                        }
                    }
                    last = Some(comparable);
                }
                if updates.changed().await.is_err() {
                    break;
                }
            }
        })
    }

    fn state(&self, current: &crate::ica::connection::ConnectionState<Client>) -> ConnectionState {
        ConnectionState {
            ready: self.ready_account(current).is_some(),
            online: current.online,
            account: current.account,
        }
    }

    fn ready_account(
        &self,
        current: &crate::ica::connection::ConnectionState<Client>,
    ) -> Option<i64> {
        let account = current.account.filter(|_| current.ready())?;
        (account == *self.expected_account.get_or_init(|| account)).then_some(account)
    }

    pub(super) fn update_rooms(
        &self,
        generation: u64,
        rooms: &[all_rooms::Room],
        full: bool,
    ) -> Option<Vec<Room>> {
        let mut snapshot = self.rooms.write().expect("外部插件房间锁被污染");
        let current = self.connection().current();
        if !current.open || current.generation != generation || (!full && snapshot.0 != generation)
        {
            return None;
        }
        let updated = rooms
            .iter()
            .map(|room| Room {
                room_id: room.room_id,
                room_name: room.room_name.clone(),
            })
            .collect::<Vec<_>>();
        *snapshot = (generation, updated.clone());
        self.ready_account(&current).map(|_| updated)
    }

    pub(super) fn rooms(
        &self,
        current: &crate::ica::connection::ConnectionState<Client>,
    ) -> Vec<Room> {
        let snapshot = self.rooms.read().expect("外部插件房间锁被污染");
        if snapshot.0 == current.generation && self.ready_account(current).is_some() {
            snapshot.1.clone()
        } else {
            Vec::new()
        }
    }

    pub(super) fn message(
        &self,
        generation: u64,
        value: &messages::NewMessage,
    ) -> Option<NewMessage> {
        let current = self.connection().current();
        if current.generation != generation {
            return None;
        }
        let account = self.ready_account(&current)?;
        let message = &value.msg;
        Some(NewMessage {
            room_id: value.room_id,
            msg_id: message.msg_id.clone(),
            sender_id: message.sender_id,
            sender_name: message.sender_name.clone(),
            role: message.role.clone(),
            content: message.content.clone(),
            time_ms: message.time.timestamp_millis(),
            is_from_self: account == message.sender_id,
            is_admin: self.admin_list().contains(&message.sender_id),
            is_reply: message.reply.is_some(),
            reply_to: message.reply.as_ref().map(reply_snapshot),
            files: message.files.iter().map(file_snapshot).collect(),
        })
    }

    fn admin_list(&self) -> &[i64] {
        self.context
            .config
            .ica
            .as_ref()
            .map_or(&[], |config| config.admin_list.as_slice())
    }

    fn current_client(&self, generation: Option<u64>) -> Result<(u64, Client), String> {
        let current = self.connection().current();
        if generation.is_some_and(|generation| generation != current.generation) {
            return Err("ICA 连接已换代，动作未发送".into());
        }
        if self.ready_account(&current).is_none() {
            return Err("ICA 连接未就绪或 QQ 账号不匹配".into());
        }
        current
            .client
            .map(|client| (current.generation, client))
            .ok_or_else(|| "ICA 连接未就绪".into())
    }
}

#[async_trait]
impl ExtHostServices for IcaServices {
    async fn hello_ack(&self) -> HelloAck {
        let current = self.connection().current();
        HelloAck {
            client_id: self.context.client_id().to_owned(),
            self_id: self.expected_account.get().copied().or(current.account),
            admin_list: self.admin_list().to_vec(),
            connection: self.state(&current),
            rooms: self.rooms(&current),
        }
    }

    async fn send_message(&self, action: SendMessage) -> Result<(), String> {
        let (generation, _) = self.current_client(None)?;
        if action.room_id == 0 {
            return Err("房间 ID 不能为 0".into());
        }
        if !self.context.config.ica().allows_room(action.room_id) {
            return Err("该房间不在宿主的 ICA 允许列表中".into());
        }
        let reply = action
            .reply_to
            .map(|id| {
                self.context
                    .read_ica(|state| {
                        state
                            .loaded_messages
                            .get_by_id(action.room_id, &id)
                            .map(messages::Message::as_reply)
                    })
                    .ok_or_else(|| "引用消息不在目标房间的已加载缓存中".to_owned())
            })
            .transpose()?;
        let mut message =
            messages::SendMessage::new(action.text.unwrap_or_default(), action.room_id, reply);
        let mut remaining = MAX_IMAGE_BYTES;
        let sticker = action.images.first().is_some_and(|image| image.as_sticker);
        for image in action.images {
            if sticker != image.as_sticker {
                return Err("同一条 ICA 消息中的图片必须使用一致的贴纸选项".into());
            }
            let (bytes, mime) = read_image(image, remaining).await?;
            remaining -= bytes.len();
            message.add_img(&bytes, &mime);
        }
        message.sticker = sticker;
        let (_, socket) = self.current_client(Some(generation))?;
        // 复用现有 Socket.IO 文本 / HTTP 媒体发送，成功只表示 Bridge 受理。
        match tokio::time::timeout(
            SEND_TIMEOUT,
            client::send_message(&socket, &message, &self.context),
        )
        .await
        {
            Ok(true) => Ok(()),
            Ok(false) => Err("Bridge 未受理消息".into()),
            Err(_) => Err("发送消息超时，无法确认 Bridge 是否受理".into()),
        }
    }

    async fn get_nt_pic_url(&self, file_id: String) -> Result<String, String> {
        let (generation, socket) = self.current_client(None)?;
        // oicq Bridge 的第二个位置参数保留，oicq 通过 file_id 本身解析资源。
        let url = client::get_nt_pic_url_by_file_id(&socket, &file_id, "").await?;
        self.current_client(Some(generation))?;
        Ok(url)
    }
}

fn reply_snapshot(reply: &messages::ReplyMessage) -> ReplyMessage {
    let files = reply
        .files
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<files::MessageFile>(value.clone()).ok())
        .map(|file| file_snapshot(&file))
        .collect();
    ReplyMessage {
        msg_id: reply.msg_id.clone(),
        sender_id: reply.raw_data.get("senderId").and_then(serde_json::Value::as_i64),
        sender_name: reply.sender_name.clone(),
        content: reply.content.clone(),
        files,
    }
}

fn file_snapshot(file: &files::MessageFile) -> MessageFile {
    MessageFile {
        file_type: file.file_type.clone(),
        url: file.url.clone(),
        fid: file.fid.clone(),
        name: file.name.clone(),
        size: file.size,
        order: file.order,
        width: file.width,
        height: file.height,
        is_face: file.is_face,
        extra: file.extra.clone(),
    }
}

async fn read_image(image: Image, remaining: usize) -> Result<(Vec<u8>, String), String> {
    if !image.mime.strip_prefix("image/").is_some_and(|subtype| {
        !subtype.is_empty()
            && subtype
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
    }) {
        return Err("图片 MIME 必须为 image/*".into());
    }
    let data = match (image.path, image.base64) {
        (Some(path), None) => {
            if !path.is_absolute() {
                return Err("图片 path 必须为本机绝对路径".into());
            }
            let metadata = tokio::fs::metadata(&path).await.map_err(|_| "无法访问图片文件")?;
            if !metadata.is_file() || metadata.len() > remaining as u64 {
                return Err("图片必须为普通文件，且图片总大小不能超过 32 MiB".into());
            }
            let file = tokio::fs::File::open(path).await.map_err(|_| "无法打开图片文件")?;
            let mut data = Vec::new();
            file.take(remaining as u64 + 1)
                .read_to_end(&mut data)
                .await
                .map_err(|_| "无法读取图片文件")?;
            data
        }
        (None, Some(encoded)) => STANDARD.decode(encoded).map_err(|_| "非法的 Base64 图片")?,
        _ => return Err("图片必须且只能指定 path 或 base64".into()),
    };
    if data.is_empty() || data.len() > remaining {
        return Err("图片不能为空，且图片总大小不能超过 32 MiB".into());
    }
    Ok((data, image.mime))
}
