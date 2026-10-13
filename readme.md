# shenbot

这是 shenjack 使用 socketio 链接某些服务后端编写的 bot 框架

## WARNING

目前 shenbot 0.9 正在进行大规模的 python 插件重写
插件 api 也有大规模修改
请使用 tag 功能 的 0.8.2 版本自行编译老版本

## 相关链接

> 其实就是一些相关项目

[本体](https://github.com/Icalingua-plus-plus/Icalingua-plus-plus)

[TODO 的客户端版本](https://github.com/shenjackyuanjie/ica-native)

[插件市场(确信)](https://github.com/shenjackyuanjie/shenbot-plugins)

## 通用环境准备

- 安装 Rust 1.88+ 和 Python；使用插件市场最新依赖时需要 Python 3.10+，推荐 Python 3.12。Windows 编译需要 Visual Studio C++ Build Tools，并在 x64 开发者命令环境中运行 Cargo。

```powershell
# 你可以使用你自己的方法安装 Python
# 例如
choco install python
# 或者
scoop install python
# 又或者
uv venv
```

- 启动 icalingua 后端

```bash
# 用你自己的方法启动你的 icalingua-bridge
# 例如
docker start icalingua
docker-compose up -d
```

## 使用方法

- 准备一个 Python 环境

- 修改好配置文件

```powershell
Copy-Item config-temp.toml config.toml
```

- 编译

```powershell
cargo build --release
```

运行

```powershell
cargo run --release -- -c config.toml
```

## oicq Bridge 插件 API

Bot 发布版本为 **0.9.4**，ICA 模块/API 版本为 **2.0.6**，Socket.IO 兼容协议为 **2.26.7**。协议以 Icalingua++ 独立 oicq Bridge 的实际实现为基准，ica-native 仅作为功能覆盖参考。

- 历史、关键词/发言人/上下文/图片查询返回可消费的 `NewMessage`，不回放新消息 hook。
- 消息提供结构化附件、引用、完整规范化 JSON 与可选原始节点链；支持更新/撤回事件、合并转发、普通/群文件上传和 fileMgr 请求。
- `request_event` 等待真实 ACK；`emit_event` 和发送方法的成功只表示传输受理，不代表 QQ 执行成功。
- 当前服务端未注册 `reportRead` / `stopFetchingHistory`，oicq `getPrivateFileUrl` 尚未实现。`report_read` 保留旧发送行为，不能据此判断 QQ 已读已上报。
- 512 KiB 上传分片以 JSON 字节数组传输，Bridge 需将 `maxHttpBufferSize` 设置为至少 4 MiB；本工作区的独立 Bridge 已同步修改。实际发送与 QQ 权限仍以服务端结果为准。

详见 [oicq Bridge API 指南](docs/oicq-bridge-api.md)；实际管理员历史/搜索/媒体插件、类型声明及离线测试在独立插件仓库同步维护。

## Python 插件运行与热重载

Python 插件仍使用原有 `shenbot_api` 接口。Bot 统一复用主 Tokio runtime；插件 hook 和生命周期函数不会持有插件注册表锁执行，避免 Python GIL、Rust 锁和异步等待互相阻塞。

插件目录默认每 1000 ms 检查一次。新增、修改或删除 `.py` 文件会在下一次扫描时加载、重载或卸载；消息到达时不再扫描磁盘。hook 函数表在加载或重载时固定，运行中修改模块属性后需要触发文件重载才能生效。每一代插件使用独立的内部模块命名空间，插件不应依赖 `__name__` 的具体字符串。

`[py]` 可选调度项及默认值：

```toml
max_concurrent_hooks = 32
max_concurrent_hooks_per_plugin = 8
max_inflight_hooks = 1024
max_inflight_hooks_per_plugin = 128
reload_interval_ms = 1000
```

执行额度满时，已接纳 hook 会等待；只有全局或单插件的在途额度耗尽时才拒绝新的 hook，并通过限频日志报告。被拒绝的 hook 不自动重试或补发。所有上限必须为正，单插件上限不能超过全局上限，在途上限不能低于对应执行上限。旧配置省略这些字段时继续使用默认值。
