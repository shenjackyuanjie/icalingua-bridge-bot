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

- 安装 Python 3.8+

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
