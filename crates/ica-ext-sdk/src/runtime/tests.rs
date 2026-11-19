use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf, duplex, split},
    task::JoinHandle,
    time::timeout,
};

use super::*;
use crate::{ShutdownReason, write_frame};

struct Harness {
    stdin: WriteHalf<DuplexStream>,
    stdout: FrameReader<BufReader<ReadHalf<DuplexStream>>>,
    task: JoinHandle<Result<(), SdkError>>,
}

impl Harness {
    fn start(handler: impl PluginHandler) -> Self {
        let (plugin, host_io) = duplex(8192);
        let (input, mut output) = split(plugin);
        let (reader, stdin) = split(host_io);
        let (outgoing, mut queue) = mpsc::channel::<Vec<u8>>(OUTBOX_CAPACITY);
        let host = HostClient::new(outgoing);
        let manifest = PluginManifest {
            id: "fixture".into(),
            name: "测试插件".into(),
            version: "0.1.0".into(),
            subscribe: Subscribe {
                commands: vec!["/test".into()],
                ..Subscribe::default()
            },
        };
        let task = tokio::spawn(async move {
            let _close = CloseClient(&host);
            let write = async {
                while let Some(frame) = queue.recv().await {
                    output.write_all(&frame).await?;
                    output.flush().await?;
                }
                Err(SdkError::Closed)
            };
            tokio::select! {
                result = drive(FrameReader::new(BufReader::new(input)), &host, manifest, handler) => result,
                result = write => result,
            }
        });
        Self {
            stdin,
            stdout: FrameReader::new(BufReader::new(reader)),
            task,
        }
    }

    /// 用原始宿主 JSON 验证扁平 v1 布局及未知字段兼容，而非同一个 serde 模型回环。
    async fn send(&mut self, frame: Value) {
        timeout(Duration::from_secs(2), write_frame(&mut self.stdin, &frame))
            .await
            .unwrap()
            .unwrap();
    }

    async fn recv(&mut self) -> Value {
        timeout(Duration::from_secs(2), self.stdout.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    async fn handshake(&mut self) {
        let hello = self.recv().await;
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["protocol_version"], 1);
        assert_eq!(hello["id"], "fixture");
        assert_eq!(hello["subscribe"]["commands"], json!(["/test"]));
        self.send(json!({
            "type": "hello_ack", "client_id": "fixture-host", "admin_list": [1],
            "self_id": 11, "connection": {"ready": true, "online": true, "account": 11},
            "rooms": [{"room_id": -1, "room_name": "测试群", "future_room_field": true}],
            "future_ack_field": 1
        }))
        .await;
    }

    async fn message(&mut self, id: &str) {
        self.send(json!({
            "type": "new_message", "room_id": -1, "msg_id": id, "sender_id": 1,
            "sender_name": "测试用户", "role": "admin", "content": "/test",
            "time_ms": 10, "is_from_self": false, "is_admin": true, "is_reply": false,
            "files": [], "future_message_field": "ignored"
        }))
        .await;
    }

    async fn reply(&mut self, request: &Value, result: Value) {
        self.send(
            json!({"type": "response", "req_id": request["req_id"], "ok": true, "result": result}),
        )
        .await;
    }

    async fn finish(mut self) {
        timeout(Duration::from_secs(2), &mut self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // 等待插件主动退出；测试在整个 drain 内保留宿主 stdin 写端。
        assert!(self.stdout.read::<Value>().await.unwrap().is_none());
    }
}

async fn echo(host: &HostClient, to: Option<String>, text: String) {
    host.send_message(SendMessage {
        room_id: -1,
        reply_to: to,
        text: Some(text),
        images: vec![],
    })
    .await
    .unwrap();
}

struct Concurrent {
    ready: AtomicBool,
}

#[async_trait::async_trait]
impl PluginHandler for Concurrent {
    async fn on_ready(&self, host: HostClient, ack: HelloAck) {
        assert_eq!(ack.client_id, "fixture-host");
        host.get_nt_pic_url("ready").await.unwrap();
        self.ready.store(true, Ordering::Release);
    }

    async fn on_message(&self, host: HostClient, msg: NewMessage) {
        assert!(self.ready.load(Ordering::Acquire));
        let text = match host.get_nt_pic_url(msg.msg_id.as_str()).await {
            Ok(url) => url,
            Err(SdkError::Rejected(reason)) => reason,
            Err(err) => panic!("动作回执意外失败：{err}"),
        };
        echo(&host, Some(msg.msg_id), text).await;
    }
}

#[tokio::test]
async fn handshake_and_concurrent_requests_match_out_of_order_responses() {
    let mut io = Harness::start(Concurrent {
        ready: AtomicBool::new(false),
    });
    io.handshake().await;
    let ready = io.recv().await;
    assert_eq!(ready["file_id"], "ready");
    io.message("one").await;
    io.message("two").await;
    io.reply(&ready, json!({"url": "ready"})).await;

    let request1 = io.recv().await;
    let request2 = io.recv().await;
    assert_eq!(request1["type"], "get_nt_pic_url");
    assert_eq!(request2["type"], "get_nt_pic_url");
    assert_ne!(request1["req_id"], request2["req_id"]);
    assert_ne!(request1["file_id"], request2["file_id"]);
    io.reply(&request2, json!({"url": "later request"})).await;
    let response2 = io.recv().await;
    assert_eq!(response2["type"], "send_message");
    assert_eq!(response2["room_id"], -1);
    assert_eq!(response2["reply_to"], request2["file_id"]);
    assert_eq!(response2["text"], "later request");
    io.reply(&response2, Value::Null).await;

    io.send(
        json!({"type": "response", "req_id": request1["req_id"], "ok": false, "error": "明确拒绝"}),
    )
    .await;
    let response1 = io.recv().await;
    assert_eq!(response1["reply_to"], request1["file_id"]);
    assert_eq!(response1["text"], "明确拒绝");
    io.reply(&response1, Value::Null).await;
    io.send(json!({"type": "shutdown", "reason": "host_exit", "grace_ms": 1000}))
        .await;
    io.finish().await;
}

struct Drain;

#[async_trait::async_trait]
impl PluginHandler for Drain {
    async fn on_message(&self, host: HostClient, msg: NewMessage) {
        assert_eq!(msg.msg_id, "existing");
        let result = host.get_nt_pic_url("existing").await.unwrap();
        echo(&host, Some(msg.msg_id), result).await;
    }

    async fn on_shutdown(&self, host: HostClient, shutdown: Shutdown) {
        assert_eq!(shutdown.reason, ShutdownReason::Reload);
        echo(&host, None, "closing".into()).await;
    }
}

#[tokio::test]
async fn shutdown_keeps_actions_and_responses_open_but_ignores_new_events() {
    let mut io = Harness::start(Drain);
    io.handshake().await;
    io.message("existing").await;
    let old_request = io.recv().await;
    io.send(json!({"type": "shutdown", "reason": "reload", "grace_ms": 1000}))
        .await;
    io.message("too-late").await;
    // shutdown 回调在旧动作还未返回时立即发新动作。
    let shutdown_action = io.recv().await;
    assert_eq!(shutdown_action["text"], "closing");
    io.reply(&shutdown_action, Value::Null).await;
    // 宿主在 grace 内继续回执；旧回调拿到回执后也能继续发动作。
    io.reply(&old_request, json!({"url": "finished during drain"}))
        .await;
    let final_action = io.recv().await;
    assert_eq!(final_action["reply_to"], "existing");
    assert_eq!(final_action["text"], "finished during drain");
    io.reply(&final_action, Value::Null).await;
    io.finish().await;
}

struct Stalled;

#[async_trait::async_trait]
impl PluginHandler for Stalled {
    async fn on_ready(&self, host: HostClient, _ack: HelloAck) {
        host.get_nt_pic_url("never answered").await.unwrap();
    }

    async fn on_shutdown(&self, host: HostClient, _shutdown: Shutdown) {
        echo(&host, None, "never answered either".into()).await;
    }
}

#[tokio::test]
async fn grace_expires_even_when_on_ready_and_shutdown_have_unanswered_actions() {
    let mut io = Harness::start(Stalled);
    io.handshake().await;
    assert_eq!(io.recv().await["file_id"], "never answered");
    io.send(json!({"type": "shutdown", "reason": "disable", "grace_ms": 200}))
        .await;
    assert_eq!(io.recv().await["text"], "never answered either");
    io.finish().await;
}

struct Overloaded;

#[async_trait::async_trait]
impl PluginHandler for Overloaded {
    async fn on_ready(&self, host: HostClient, _ack: HelloAck) {
        host.get_nt_pic_url("ready gate").await.unwrap();
    }

    async fn on_message(&self, host: HostClient, message: NewMessage) {
        assert!(!message.msg_id.starts_with("overflow"), "过载事件不应运行");
        host.get_nt_pic_url(message.msg_id).await.unwrap();
    }

    async fn on_shutdown(&self, host: HostClient, _shutdown: Shutdown) {
        echo(&host, None, "closing while overloaded".into()).await;
    }
}

#[tokio::test]
async fn overload_drops_events_without_blocking_responses_or_shutdown() {
    let mut io = Harness::start(Overloaded);
    io.handshake().await;
    let ready = io.recv().await;
    for id in 0..MAX_EVENTS {
        io.message(&id.to_string()).await;
    }
    io.message("overflow-before-ready").await;
    io.reply(&ready, json!({"url": "ready"})).await;

    let mut requests = Vec::new();
    for _ in 0..MAX_EVENTS {
        requests.push(io.recv().await);
    }
    io.message("overflow-while-running").await;
    io.send(json!({"type": "shutdown", "reason": "disable", "grace_ms": 1000}))
        .await;
    let shutdown_action = io.recv().await;
    assert_eq!(shutdown_action["text"], "closing while overloaded");
    io.reply(&shutdown_action, Value::Null).await;
    for request in requests {
        io.reply(&request, json!({"url": "finished"})).await;
    }
    io.finish().await;
}
