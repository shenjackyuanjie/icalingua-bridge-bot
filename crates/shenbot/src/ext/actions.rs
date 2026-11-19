//! 外部插件的动作与回执。实际 Bridge 调用由中立宿主注入。

use std::sync::{Arc, Weak};
use std::time::Duration;

use ica_ext_sdk::{HelloAck, HostToPlugin, PluginToHost, Response, SendMessage, async_trait};
use serde_json::json;
use tokio::time::timeout;

use super::lifecycle::Instance;
use super::process::PluginStdin;

#[async_trait]
pub trait ExtHostServices: Send + Sync {
    /// 返回当前 ICA 连接状态与群列表快照。
    async fn hello_ack(&self) -> HelloAck;
    async fn send_message(&self, message: SendMessage) -> Result<(), String>;
    async fn get_nt_pic_url(&self, file_id: String) -> Result<String, String>;
}

pub(super) struct ActionContext {
    pub(super) plugin_id: String,
    pub(super) instance: Weak<Instance>,
    pub(super) run: Arc<()>,
    pub(super) stdin: PluginStdin,
    pub(super) services: Arc<dyn ExtHostServices>,
    pub(super) write_timeout: Duration,
}

impl ActionContext {
    pub(super) async fn execute(&self, action: PluginToHost) {
        match action {
            PluginToHost::SendMessage { req_id, message } => {
                let result = match validate_message(&message) {
                    Ok(()) => self.services.send_message(message).await,
                    Err(error) => Err(error),
                };
                self.respond(req_id, result.map(|()| None)).await;
            }
            PluginToHost::GetNtPicUrl { req_id, file_id } => {
                let result = if file_id.trim().is_empty() {
                    Err("图片文件 ID 不能为空".into())
                } else {
                    self.services
                        .get_nt_pic_url(file_id)
                        .await
                        .map(|url| Some(json!({ "url": url })))
                };
                self.respond(req_id, result).await;
            }
            PluginToHost::RequestReload { req_id, .. } => {
                let reservation = match self.instance.upgrade() {
                    Some(instance) => instance
                        .reserve_self_reload(&self.run)
                        .map(|guard| (instance, guard))
                        .map_err(|error| error.to_string()),
                    None => Err("外部插件宿主已退出".into()),
                };
                match reservation {
                    Ok((instance, guard)) => {
                        // 插件回调可能正等待这个回执；先实际写入，再让新任务开始排空旧代。
                        if self.respond(req_id, Ok(None)).await {
                            instance.self_reload(guard);
                        }
                    }
                    Err(error) => {
                        self.respond(req_id, Err(error)).await;
                    }
                }
            }
            PluginToHost::Hello(_) => unreachable!("运行期间的 hello 由监管器拒绝"),
        }
    }

    async fn respond(
        &self,
        req_id: u64,
        result: Result<Option<serde_json::Value>, String>,
    ) -> bool {
        let response = match result {
            Ok(result) => Response {
                req_id,
                ok: true,
                result,
                error: None,
            },
            Err(error) => Response {
                req_id,
                ok: false,
                result: None,
                error: Some(error),
            },
        };
        let frame = HostToPlugin::Response(response);
        match timeout(self.write_timeout, self.stdin.send(&frame)).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::debug!(plugin_id = %self.plugin_id, req_id, %error, "外部插件动作回执写入失败");
                false
            }
            Err(_) => {
                tracing::warn!(plugin_id = %self.plugin_id, req_id, "写入外部插件动作回执超时");
                false
            }
        }
    }
}

fn validate_message(message: &SendMessage) -> Result<(), String> {
    if message.text.as_ref().is_none_or(String::is_empty) && message.images.is_empty() {
        return Err("发送的消息必须包含文字或图片".into());
    }
    for image in &message.images {
        let has_source = match (&image.path, &image.base64) {
            (Some(path), None) => !path.as_os_str().is_empty(),
            (None, Some(data)) => !data.is_empty(),
            _ => false,
        };
        if !has_source || image.mime.trim().is_empty() {
            return Err("图片必须包含 MIME，并且只能提供一个非空的 path 或 base64".into());
        }
    }
    Ok(())
}
