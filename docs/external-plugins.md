# ICA 外部进程插件

Shenbot 可以同时运行 Python 插件和外部可执行插件，也可以只启用外部插件。外部插件第一版接入 **Socket.IO + oicq Bridge**，不接收 Tailchat 的消息。宿主在独立进程执行插件；**这不是沙箱**，只配置来源可信、可在宿主账号下执行的程序。

## 启用插件

在宿主主配置的顶层只需开启：

```toml
enable_ext = true
```

将 [ds_monitor.plugins.toml 示例](examples/ds_monitor.plugins.toml) 复制到插件目录并调整路径。外部插件与 `.py` 脚本共用 `[py].plugin_path` 配置指定的目录；不设置时使用 `./plugins`。宿主只扫描目录直属的 `*.plugins.toml`，不执行其他 TOML 文件。

- 文件名决定插件 ID，如 `ds_monitor.plugins.toml` 对应 `ds_monitor`。ID 区分大小写，使用 1–64 个 ASCII 字母、数字、下划线或连字符。外部 unit（包括未启用的）先占用 ID，Python 扫描跳过同 ID 插件。
- unit 中的相对 `binary`、`working_dir` 以 unit 所在目录解析；不写 `working_dir` 时也使用该目录。`args` 按顺序原样传递，不经 shell。`[ext]` 仅可选配置 `shadow_dir`、`handshake_timeout_ms`、`default_drain_timeout_ms`；相对 `shadow_dir` 仍以宿主工作目录解析。
- `enabled`、`auto_restart`、`allow_self_reload` 都默认关闭。
- 宿主启动时读取 unit；新增、删除或编辑后重启宿主生效。`/bot-reload-<client-id> <插件ID>` 只重载可执行文件，不重读 unit。`/bot-ls` 显示 `[ext]` 和状态，`/bot-help` 提供管理命令；启停命令只调整本次运行，不写回 unit。
- 同时保留 `[ica]` 配置。多个宿主连接同一个 Bridge 时，可给测试实例配置 `[ica].allow_rooms`（群 ID 为负数），限制实时消息、内置命令和外部插件发送；隔离测试请关闭 Python，因为此项不限制 Python 插件主动调用 Bridge。

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
