# ICA 外部进程插件

Shenbot 可以同时运行 Python 插件和外部可执行插件，也可以只启用外部插件。外部插件第一版接入 **Socket.IO + oicq Bridge**，不接收 Tailchat 的消息。宿主在独立进程执行插件；**这不是沙箱**，只配置来源可信、可在宿主账号下执行的程序。

## 启用插件

在宿主的 TOML 配置中启用插件，先编译并安装插件可执行文件。示例中的路径不引用运行副本：

```toml
enable_ica = true
enable_py = false
enable_ext = true

[ext]
shadow_dir = "./.ext_runtime"
handshake_timeout_ms = 5000
default_drain_timeout_ms = 30000

[[ext.plugins]]
id = "ds_monitor"
enabled = true
binary = 'D:\apps\ds-monitor\target\release\ds-monitor.exe'
args = ["serve-ica", "--config", 'D:\apps\ds-monitor\config.toml']
working_dir = 'D:\apps\ds-monitor'
reload_mode = "serial"
drain_timeout_ms = 120000
auto_restart = true
allow_self_reload = true
```

- 同时保留完整的 `[ica]` 配置。多个宿主连接同一个 Bridge 时，可给测试实例配置 `[ica].allow_rooms`（群 ID 为负数），限制实时消息、内置命令和外部插件发送；这不限制 Python 插件主动调用 Bridge，隔离测试请关闭 Python。不要在同一个宿主中同时启用同 ID 的新旧插件。外部配置清单占用全部 ID（含已禁用项）；Python 扫描不会加载冲突项。
- 相对的 `binary`、`working_dir`、`shadow_dir` 从宿主工作目录解析。`args` 按顺序原样传递，不经 shell；插件自行解析参数。外部 ID 区分大小写，使用 1–64 个 ASCII 字母、数字、下划线或连字符。
- 未配置 `enable_ext` 时保持禁用；启用时需提供 `[ext]`。未指定 `enabled`、`auto_restart` 或 `allow_self_reload` 的插件默认不启用相应行为。握手和排空超时范围为 1 毫秒到 24 小时。
- 从 QQ 发送 `/bot-ls` 查看 `[py]`、`[ext]` 及状态。管理员可使用 `/bot-enable-<client-id> <插件ID>`、`/bot-disable-<client-id> <插件ID>`、`/bot-reload-<client-id> <插件ID>`；`/bot-help` 展示当前宿主编号。命令调整的是本次运行状态，不写回 TOML。

## 更新与停机

宿主为每次启动/更新保存独立的可执行文件副本，更新时编译覆盖配置中指向的**源二进制**，不要手工替换运行副本。Windows 下宿主将子进程绑入 Job Object，关闭宿主会清理受管进程树；启动时安全清扫上次遗留的副本。

- `serial`（默认）：先复制新版，再停止给旧代派新事件并让旧代排空；启动新版握手失败时用保留的旧副本回滚。写同一份本地数据的插件用此模式。
- `overlap`：新版握手成功后才切换路由。旧代仍可在排空期限内发动作并收回执，失败的新版不会中断旧代。只有能安全并行的插件使用此模式。
- `auto_restart` 指数退避重启（1、2、4、8、16 秒，上限 16 秒）；连续 30 秒窗口内第 5 次故障熔断。修复后通过管理员命令显式重载；不要用定时任务持续重启宿主来绕过熔断。
- 自热切需显式配置 `allow_self_reload`。动作成功回执表示已受理热切请求，**不是新版启动成功的确认**；在 `/bot-ls` 或宿主日志中查看结果。宿主退出在关闭 Bridge 前先排空插件，二次 Ctrl+C 时强制退出。

## 开发 Rust 插件

使用 [ica-ext-sdk](../crates/ica-ext-sdk/) 实现 `PluginHandler`，在异步入口调用 `ica_ext_sdk::run`。清单 ID 要与宿主配置相同。日志用 stderr（宿主按插件归集），**不要向 stdout 打印普通文本**。

- stdin 和 stdout 各自承载一行一条的 UTF-8 JSON，`type` 区分消息；协议版本为 1，单行上限为 16 MiB。未识别的对象字段忽略。插件先发 `hello`，收到 `hello_ack` 后开始使用快照及动作接口。
- 在 `subscribe.commands` 中只声明需要的命令前缀，宿主在派发前过滤；只有真正需要时开启全量消息、系统消息、连接或房间事件。历史/缓存加载不会作为实时消息回放。
- `send_message` 要有文字或图片；附件只指定一个 Base64 或本机绝对路径，图片合计最多 32 MiB（Base64 仍受单行上限），同一消息的贴纸选项需一致。命令引用按目标房间的当前缓存匹配，找不到时返回错误，不改为错误房间的引用。文本成功回执表示 Socket.IO 接受了发送，含图片的 HTTP 成功回执表示 Bridge 返回 202；两者均不保证 QQ 送达。超时不要盲目重试。`get_nt_pic_url` 只请求 Bridge 获取新地址，不代为下载。
- 收到 `shutdown` 后停止接新任务，在宽限期内结束在途动作。SDK 会等待其管理的回调，插件自行派生的后台工作应由 `on_shutdown` 收尾。切勿在 stdout/管道中输出私密配置。

## 联调

`cargo test -p shenbot --test ext_plugins` 启动真实插件子进程，不需要 QQ。若本机已安装 Node.js 与 `socket.io`，也可运行[环回 Bridge 冒烟](../crates/shenbot/tests/ext_plugins/bridge_smoke.cjs)：先运行 `cargo build --bin shenbot` 和 `cargo test -p shenbot --test ext_plugins --no-run`，再向脚本传入 `socket.io` 入口、宿主产物及测试可执行文件的路径。脚本使用临时配置并且只连接 `127.0.0.1`。
