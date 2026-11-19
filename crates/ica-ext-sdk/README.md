# ica-ext-sdk

ica-bot 原生外部进程插件 SDK。运行 `cargo build -p ica-ext-sdk --example echo` 可构建[最小回显示例](examples/echo.rs)。它订阅 `/echo` 前缀，并在原房间引用回复命令内容。

## 插件接口

- 实现 `PluginHandler`，并用 `run(manifest, handler).await` 启动；每个进程只运行一次。标准输出专用于 NDJSON，日志写入标准错误。
- `on_ready` 接收握手快照，可以发动作。它完成后才派发普通事件。事件回调可以并发，不能依赖回调的完成顺序。SDK 最多缓存或并发运行 32 个业务事件；满时丢弃新业务事件并在 stderr 提示一次，回执和停机指令照常处理。
- `HostClient` 可以克隆；每个动作等待自己的回执。SDK 不自动超时或重发；本地取消等待、超时或 `SdkError::Closed` 都不能证明宿主没有执行。`SdkError::Rejected` 表示宿主明确返回失败回执。
- 收到 `shutdown` 后不再派发新事件，立即与已开始的回调并行执行 `on_shutdown`。宽限期内双方继续交换动作与回执。若插件自行派生后台任务，应在 `on_shutdown` 里等待它们；SDK 等待已跟踪回调结束或宽限到期。

## 宿主接口

- 双方使用[线协议类型](src/protocol.rs)；消息由顶层 `type` 区分。正文限制为 16 MiB，不含 LF 或 CRLF；使用 `FrameReader` 和 `encode_frame` 共用边界检查。
- `send_message.reply_to` 是当前房间里的目标消息 ID。图片传本机路径优先；收到引用图片时可以用附件 `fid` 调用 `get_nt_pic_url`。
- 成功的 `get_nt_pic_url` 回执在 `result` 中返回 `{"url":"…"}`。`request_reload` 的成功回执只表示受理；宿主应先把回执交给旧代，再启动热切，不能等旧代退出后才回执。
