//! 最小原生插件：订阅 /echo，把命令内容原样作为引用回复。

use ica_ext_sdk::{
    HostClient, NewMessage, PluginHandler, PluginManifest, SdkError, SendMessage, Subscribe,
    async_trait, run,
};

struct Echo;

#[async_trait]
impl PluginHandler for Echo {
    async fn on_message(&self, host: HostClient, message: NewMessage) {
        let Err(err) = host
            .send_message(SendMessage {
                room_id: message.room_id,
                reply_to: Some(message.msg_id),
                text: Some(message.content),
                images: vec![],
            })
            .await
        else {
            return;
        };
        eprintln!("echo 回复失败：{err}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), SdkError> {
    run(
        PluginManifest {
            id: "echo".into(),
            name: "最小回显插件".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            subscribe: Subscribe {
                commands: vec!["/echo".into()],
                ..Subscribe::default()
            },
        },
        Echo,
    )
    .await
}
