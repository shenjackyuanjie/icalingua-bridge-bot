//! 同一个测试可执行文件在显式子进程模式下也是真实 SDK 插件。

use std::path::PathBuf;
use std::time::Duration;

use ica_ext_sdk::{
    HelloAck, HostClient, Image, NewMessage, PluginHandler, PluginManifest, SendMessage, Shutdown,
    Subscribe, async_trait, run,
};

pub const PLUGIN_ID: &str = "ext_fixture";
pub const ROOM_ID: i64 = -42;
pub const PIC_ID: &str = "fake-nt-image";
pub const PIC_URL: &str = "https://fixture.invalid/image";

pub async fn run_plugin(mode: String, directory: PathBuf) {
    let current = std::env::current_exe().expect("读取运行副本路径");
    let is_second_generation = current
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with("-gen2"));
    let bad = is_second_generation && matches!(mode.as_str(), "serial" | "overlap");
    if bad && mode == "serial" {
        // 新代已经从副本启动；破坏配置的源，确保自动回滚只能重用旧代副本。
        std::fs::write(directory.join("source.exe"), b"unavailable").expect("移除可用的源文件");
    }
    let manifest = PluginManifest {
        id: if bad { "wrong_id" } else { PLUGIN_ID }.into(),
        name: "外部插件测试程序".into(),
        version: "test".into(),
        subscribe: Subscribe {
            commands: vec!["/fixture ".into()],
            ..Default::default()
        },
    };
    run(manifest, Fixture { mode, directory }).await.expect("测试插件协议运行失败");
}

struct Fixture {
    mode: String,
    directory: PathBuf,
}

#[async_trait]
impl PluginHandler for Fixture {
    async fn on_ready(&self, host: HostClient, _ack: HelloAck) {
        if self.mode == "crash" {
            emit(&host, "boot").await;
            std::process::exit(9);
        }
        #[cfg(windows)]
        if self.mode == "tree" {
            use std::process::Stdio;
            let mut descendant =
                tokio::process::Command::new(std::env::current_exe().expect("读取副本路径"))
                    .arg("--ext-descendant")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("启动 Job Object 内的后代进程");
            emit(&host, &format!("descendant:{}", descendant.id().expect("获得后代进程号"))).await;
            tokio::spawn(async move {
                let _ = descendant.wait().await;
            });
        }
    }

    async fn on_message(&self, host: HostClient, message: NewMessage) {
        match message.content.as_str() {
            "/fixture ping" => emit(&host, "pong").await,
            "/fixture picture" => {
                let url = host.get_nt_pic_url(PIC_ID).await.expect("取得图片动作回执");
                emit(&host, &url).await;
            }
            "/fixture image" => {
                host.send_message(SendMessage {
                    room_id: ROOM_ID,
                    reply_to: Some("fixture-message".into()),
                    text: None,
                    images: vec![Image {
                        path: Some(self.directory.join("fixture-image.png")),
                        base64: None,
                        mime: "image/png".into(),
                        as_sticker: false,
                    }],
                })
                .await
                .expect("取得 HTTP 图片受理回执");
                emit(&host, "image-acked").await;
            }
            "/fixture reload" => {
                let tag = if host.request_reload(None).await.is_ok() {
                    "reload-accepted"
                } else {
                    "reload-denied"
                };
                emit(&host, tag).await;
            }
            "/fixture hold" => {
                emit(&host, "holding").await;
                while !self.directory.join("release").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                emit(&host, "old-work").await;
                emit(&host, "old-work-acked").await;
            }
            _ => emit(&host, "unexpected-routing").await,
        }
    }

    async fn on_shutdown(&self, host: HostClient, _shutdown: Shutdown) {
        if self.mode == "tree" {
            std::future::pending::<()>().await;
        }
        emit(&host, "shutdown").await;
    }
}

async fn emit(host: &HostClient, tag: &str) {
    host.send_message(SendMessage {
        room_id: ROOM_ID,
        reply_to: Some("fixture-message".into()),
        text: Some(format!("{}:{tag}", std::process::id())),
        images: Vec::new(),
    })
    .await
    .expect("取得发送消息动作回执");
}
