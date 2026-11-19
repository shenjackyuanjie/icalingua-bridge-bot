use std::{
    collections::HashMap,
    io::{Read, Write},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    thread,
    time::Duration,
};

use tokio::{
    io::{AsyncBufRead, AsyncRead, BufReader, ReadBuf},
    sync::{mpsc, oneshot},
    task::JoinSet,
    time::sleep,
};

use crate::{
    ConnectionState, FrameReader, Hello, HelloAck, HostToPlugin, NewMessage, PROTOCOL_VERSION,
    PluginToHost, Response, Room, SdkError, SendMessage, Shutdown, Subscribe, encode_frame,
};

const OUTBOX_CAPACITY: usize = 8;
const MAX_EVENTS: usize = 32;

#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub subscribe: Subscribe,
}

/// 消息及状态回调可以并发。待派发或运行中的业务事件最多 32 个；超量时丢弃新业务事件。
/// 退场时 on_shutdown 与已开始的回调同时运行；
/// 若插件自行派生后台任务，应在 on_shutdown 中等待它们，才能继续收到动作回执。
#[async_trait::async_trait]
pub trait PluginHandler: Send + Sync + 'static {
    async fn on_ready(&self, _host: HostClient, _ack: HelloAck) {}
    async fn on_message(&self, _host: HostClient, _message: NewMessage) {}
    async fn on_connection_state(&self, _host: HostClient, _state: ConnectionState) {}
    async fn on_rooms_updated(&self, _host: HostClient, _rooms: Vec<Room>) {}
    async fn on_shutdown(&self, _host: HostClient, _shutdown: Shutdown) {}
}

/// 可跨回调共享的动作客户端。动作不自动超时或重发；取消等待不撤销宿主动作。
#[derive(Clone)]
pub struct HostClient {
    inner: Arc<ClientInner>,
}

#[derive(Default)]
struct Pending {
    closed: bool,
    last_id: u64,
    requests: HashMap<u64, oneshot::Sender<Response>>,
}

struct ClientInner {
    outgoing: mpsc::Sender<Vec<u8>>,
    pending: Mutex<Pending>,
}

/// 取消动作 future 时清理本地等待者，不会撤回已写出的动作。
struct RequestGuard<'a> {
    id: u64,
    pending: &'a Mutex<Pending>,
}

impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.pending.lock().unwrap().requests.remove(&self.id);
    }
}

impl HostClient {
    fn new(outgoing: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            inner: Arc::new(ClientInner {
                outgoing,
                pending: Mutex::new(Pending::default()),
            }),
        }
    }

    pub async fn send_message(&self, message: SendMessage) -> Result<(), SdkError> {
        self.request(|req_id| PluginToHost::SendMessage { req_id, message })
            .await?;
        Ok(())
    }

    pub async fn get_nt_pic_url(&self, file_id: impl Into<String>) -> Result<String, SdkError> {
        let file_id = file_id.into();
        let response = self
            .request(|req_id| PluginToHost::GetNtPicUrl { req_id, file_id })
            .await?;
        response
            .result
            .as_ref()
            .and_then(|result| result.get("url"))
            .and_then(|url| url.as_str())
            .map(str::to_owned)
            .ok_or(SdkError::InvalidResponse("get_nt_pic_url"))
    }

    /// 成功表示宿主已受理热切，不代表新代插件已经就绪。
    pub async fn request_reload(&self, reason: Option<String>) -> Result<(), SdkError> {
        self.request(|req_id| PluginToHost::RequestReload { req_id, reason })
            .await?;
        Ok(())
    }

    async fn request(&self, build: impl FnOnce(u64) -> PluginToHost) -> Result<Response, SdkError> {
        let id = {
            let mut pending = self.inner.pending.lock().unwrap();
            if pending.closed {
                return Err(SdkError::Closed);
            }
            pending.last_id = pending
                .last_id
                .checked_add(1)
                .ok_or(SdkError::RequestIdsExhausted)?;
            pending.last_id
        };
        // 在编码前取得有界出站队列的槽位，避免无界积压编码后的大帧。
        let slot = self
            .inner
            .outgoing
            .reserve()
            .await
            .map_err(|_| SdkError::Closed)?;
        let frame = encode_frame(&build(id))?;
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().unwrap();
            if pending.closed {
                return Err(SdkError::Closed);
            }
            pending.requests.insert(id, tx);
            slot.send(frame);
        }
        let _guard = RequestGuard {
            id,
            pending: &self.inner.pending,
        };
        let response = rx.await.map_err(|_| SdkError::Closed)?;
        if response.ok {
            Ok(response)
        } else {
            Err(SdkError::Rejected(
                response.error.unwrap_or_else(|| "宿主未提供原因".into()),
            ))
        }
    }

    fn respond(&self, response: Response) {
        let waiter = self
            .inner
            .pending
            .lock()
            .unwrap()
            .requests
            .remove(&response.req_id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(response);
        }
    }

    fn close(&self) {
        let mut pending = self.inner.pending.lock().unwrap();
        pending.closed = true;
        pending.requests.clear();
    }
}

/// 运行插件。使用独立标准线程处理阻塞 stdio，退出时不等待宿主先关闭 stdin，
/// 也不把无法取消的系统管道读写挂在 Tokio 运行时的阻塞线程池上。
pub async fn run(manifest: PluginManifest, handler: impl PluginHandler) -> Result<(), SdkError> {
    let (input, chunks) = mpsc::channel(2);
    let stdin = StdinChunks {
        chunks,
        current: Vec::new(),
        offset: 0,
    };
    thread::Builder::new()
        .name("ica-plugin-stdin".into())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            loop {
                let mut bytes = [0; 8192];
                let chunk = match stdin.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => Ok(bytes[..n].to_vec()),
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => Err(err),
                };
                let failed = chunk.is_err();
                if input.blocking_send(chunk).is_err() || failed {
                    break;
                }
            }
        })?;

    let (outgoing, mut outbox) = mpsc::channel::<Vec<u8>>(OUTBOX_CAPACITY);
    let (writer_result, writer_done) = oneshot::channel();
    thread::Builder::new()
        .name("ica-plugin-stdout".into())
        .spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let mut stdout = std::io::stdout().lock();
                while let Some(frame) = outbox.blocking_recv() {
                    if writer_result.is_closed() {
                        break;
                    }
                    stdout.write_all(&frame)?;
                    stdout.flush()?;
                }
                Ok(())
            })();
            let _ = writer_result.send(result);
        })?;

    let host = HostClient::new(outgoing);
    let _close = CloseClient(&host);
    tokio::select! {
        result = drive(FrameReader::new(BufReader::new(stdin)), &host, manifest, handler) => result,
        result = writer_done => match result {
            Ok(Err(err)) => Err(err.into()),
            _ => Err(SdkError::Closed),
        },
    }
}

struct CloseClient<'a>(&'a HostClient);

impl Drop for CloseClient<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct StdinChunks {
    chunks: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    current: Vec<u8>,
    offset: usize,
}

impl AsyncRead for StdinChunks {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dest: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if dest.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.offset == self.current.len() {
            match std::task::ready!(self.chunks.poll_recv(cx)) {
                Some(Ok(bytes)) => {
                    self.current = bytes;
                    self.offset = 0;
                }
                Some(Err(err)) => return Poll::Ready(Err(err)),
                None => return Poll::Ready(Ok(())),
            }
        }
        let end = (self.offset + dest.remaining()).min(self.current.len());
        dest.put_slice(&self.current[self.offset..end]);
        self.offset = end;
        Poll::Ready(Ok(()))
    }
}

#[derive(PartialEq, Eq)]
enum CallbackResult {
    Ready,
    Event,
}

enum Event {
    Message(Box<NewMessage>),
    Connection(ConnectionState),
    Rooms(Vec<Room>),
}

fn spawn_event(
    callbacks: &mut JoinSet<CallbackResult>,
    handler: &Arc<impl PluginHandler>,
    host: &HostClient,
    event: Event,
) {
    let (handler, host) = (handler.clone(), host.clone());
    callbacks.spawn(async move {
        match event {
            Event::Message(message) => handler.on_message(host, *message).await,
            Event::Connection(state) => handler.on_connection_state(host, state).await,
            Event::Rooms(rooms) => handler.on_rooms_updated(host, rooms).await,
        }
        CallbackResult::Event
    });
}

async fn drive(
    mut reader: FrameReader<impl AsyncBufRead + Unpin>,
    host: &HostClient,
    manifest: PluginManifest,
    handler: impl PluginHandler,
) -> Result<(), SdkError> {
    let hello = PluginToHost::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        subscribe: manifest.subscribe,
    });
    host.inner
        .outgoing
        .send(encode_frame(&hello)?)
        .await
        .map_err(|_| SdkError::Closed)?;
    let Some(HostToPlugin::HelloAck(ack)) = reader.read().await? else {
        return Err(SdkError::ExpectedHelloAck);
    };

    let handler = Arc::new(handler);
    let mut callbacks = JoinSet::new();
    {
        let (handler, host) = (handler.clone(), host.clone());
        callbacks.spawn(async move {
            handler.on_ready(host, ack).await;
            CallbackResult::Ready
        });
    }
    let mut ready = false;
    let mut deferred = Vec::new();
    let mut warned_overload = false;
    loop {
        tokio::select! {
            biased;
            Some(result) = callbacks.join_next(), if !callbacks.is_empty() => {
                if result.map_err(SdkError::Callback)? == CallbackResult::Ready {
                    ready = true;
                    for event in deferred.drain(..) {
                        spawn_event(&mut callbacks, &handler, host, event);
                    }
                }
            }
            message = reader.read::<HostToPlugin>() => {
                let event = match message? {
                    Some(HostToPlugin::NewMessage(message)) => Event::Message(Box::new(message)),
                    Some(HostToPlugin::ConnectionState(state)) => Event::Connection(state),
                    Some(HostToPlugin::RoomsUpdated { rooms }) => Event::Rooms(rooms),
                    Some(HostToPlugin::Response(response)) => {
                        host.respond(response);
                        continue;
                    }
                    Some(HostToPlugin::Shutdown(shutdown)) => {
                        drop(deferred);
                        return drain(&mut reader, host, handler, callbacks, shutdown).await;
                    }
                    Some(HostToPlugin::HelloAck(_)) => continue,
                    None => return Err(SdkError::Closed),
                };
                let occupied = if ready { callbacks.len() } else { deferred.len() };
                if occupied >= MAX_EVENTS {
                    if !warned_overload {
                        warned_overload = true;
                        let _ = writeln!(std::io::stderr(), "ica-ext-sdk：业务事件积压达到 {MAX_EVENTS}，丢弃新事件；动作回执和停机指令仍会处理");
                    }
                } else if ready {
                    spawn_event(&mut callbacks, &handler, host, event);
                } else {
                    deferred.push(event);
                }
            }
        }
    }
}

async fn drain(
    reader: &mut FrameReader<impl AsyncBufRead + Unpin>,
    host: &HostClient,
    handler: Arc<impl PluginHandler>,
    mut callbacks: JoinSet<CallbackResult>,
    shutdown: Shutdown,
) -> Result<(), SdkError> {
    let deadline = sleep(Duration::from_millis(shutdown.grace_ms));
    tokio::pin!(deadline);
    let context = host.clone();
    callbacks.spawn(async move {
        handler.on_shutdown(context, shutdown).await;
        CallbackResult::Event
    });
    let mut input_open = true;
    while !callbacks.is_empty() {
        tokio::select! {
            biased;
            _ = &mut deadline => break,
            Some(result) = callbacks.join_next() => { result.map_err(SdkError::Callback)?; }
            message = reader.read::<HostToPlugin>(), if input_open => match message? {
                Some(HostToPlugin::Response(response)) => host.respond(response),
                Some(_) => {},
                None => {
                    host.close();
                    input_open = false;
                }
            }
        }
    }
    // JoinSet 的 Drop 会取消尚未结束的回调。
    Ok(())
}

#[cfg(test)]
mod tests;
