# 更新日志 (0.2 ~ 0.8)

## 0.8.2

- ica 兼容版本号更新到 `2.12.28`
- 现在支持通过读取环境变量里的 `VIRTUAL_ENV` 来获取虚拟环境路径
  - 用于在虚拟环境中运行插件
- 添加了 `-h` 参数
  - 用于展示帮助信息
- 添加了 `-env` 参数
  - 用于指定 python 插件的虚拟环境路径
  - 会覆盖环境变量中的 `VIRTUAL_ENV`
- 现在加入了默认的配置文件路径 `./config.toml`
- 现在会记录所有的 python 运行中 task 了
  - 也会在退出的时候等待所有的 task 结束
  - 二次 ctrl-c 会立即退出
- 改进了一下 ica 的新消息显示
- 添加了 ica 链接用时的显示

### ica&tailchat 2.0.0

- BREAKING CHANGE
  - 现在 `CONFIG_DATA` 为一个 `str | bytes`
  - 用于存储插件的配置信息
  - 需要自行解析
  - 现在 `on_config` 函数签名为 `on_config = Callable[[bytes], None]`
    - 用于接收配置信息
  - 现在使用 `require_config = Callable[[None], str, bytes | str]` 函数来请求配置信息
    - 用于请求配置信息
    - `str` 为配置文件名
    - `bytes | str` 为配置文件默认内容
      - 以 `bytes` 形式或者 `str` 形式均可

### ica 1.6.7

- 为 `IcaClinet` 添加了 `py_tasks_count -> int` 属性
  - 用于获取当前运行中的 python task 数量

### tailchat 1.2.6

- 为 `TailchatClient` 添加了 `py_tasks_count -> int` 属性
  - 用于获取当前运行中的 python task 数量

## 0.8.1

- 修复了 Python 插件状态写入的时候写入路径错误的问题
- `ica-typing` 加入了 `from __future__ import annotations`
  - 这样就可以随便用 typing 了
  - 把 NewType 都扬了

### ica 1.6.6

- 修复了 `send_poke` api 的问题
  - 现在可以正常使用了

## 0.8.0

- ica 兼容版本号更新到 ~~`2.12.24`~~ `2.12.26`
- 从 `py::PyStatus` 开始进行一个 `static mut` -> `static mut OnceLock` 的改造
  - 用于看着更舒服(逃)
- 部分重构了一下 读取插件启用状态 的配置文件的代码
- 现在 `/bot-help` 会直接输出实际的 client id, 而不是给你一个默认的 `<client-id>`

### ica 1.6.5

- 添加了 `send_room_sign_in` api
  - 用于发送群签到信息
  - socketio event: `sendGroupSign`
- 添加了 `send_poke` api
  - 用于发送戳一戳
  - 可以指定群的某个人
  - 或者指定好友
  - 目前还是有点问题
  - socketio event: `sendGroupPoke`
- 添加了 `reload_plugin_status` api
  - 用于重新加载插件状态
- 添加了 `reload_plugin(plugin_name: str)` api
  - 用于重新加载指定插件
- 添加了 `set_plugin_status(plugin_name: str, status: bool)` api
  - 用于设置插件的启用状态
- 添加了 `get_plugin_status(plugin_name: str) -> bool` api
  - 用于获取插件的启用状态
- 添加了 `sync_status_to_config` api
  - 用于将内存中的插件状态同步到配置文件中

### tailchat 1.2.5

- 添加了 `reload_plugin_status` api
  - 用于重新加载插件状态
- 添加了 `reload_plugin(plugin_name: str)` api
  - 用于重新加载指定插件
- 添加了 `set_plugin_status(plugin_name: str, status: bool)` api
  - 用于设置插件的启用状态
- 添加了 `get_plugin_status(plugin_name: str) -> bool` api
  - 用于获取插件的启用状态
- 添加了 `sync_status_to_config` api
  - 用于将内存中的插件状态同步到配置文件中

## 0.7.4

- ica 兼容版本号更新到 `2.12.23`
- 通过一个手动 json patch 修复了因为 icalingua 的奇怪类型问题导致的 bug
- [icalingua issue](https://github.com/Icalingua-plus-plus/Icalingua-plus-plus/issues/793)

## 0.7.3

- 也许修复了删除插件不会立即生效的问题
- ica 兼容版本号更新到 `2.12.21`
添加了一些新的 api

### ica 1.6.4

- 给 `SendMessagePy`
  - 添加了 `remove_reply` 方法
  - 用于取消回复状态
- 删除了 `Room` 的 `auto_download` 和 `download_path` 字段
  - 因为这两个字段也没啥用

### tailcaht 1.2.4

- 给 `TailchatClientPy`
  - 添加了 `new_message` 方法
  - 用于创建新的消息
- 给 `TailchatSendingMessagePy`
  - 添加了 `clear_meta` 功能
  - 用于清除 `meta` 字段
  - 可以用来取消回复状态

## 0.7.2

- 修复了一些 ica 和 tailchat 表现不一致的问题(捂脸)

## 0.7.1

- 两个 api 版本号分别升级到 `1.6.3(ica)` 和 `1.2.3(tailchat)`
- 加入了 `client_id`
  - 用的 startup time hash 一遍取后六位
  - 以及也有 python 侧的 `client_id` api
- 修复了上个版本其实没有写 python 侧 `version_str` api 的问题

## 0.7.0

> 我决定叫他 0.7.0
> 因为修改太多了.png

- 加入了 禁用/启用 插件功能
- 现在会在插件加载时警告你的插件原来定义了 `CONFIG_DATA` 这一项
- `IcaNewMessage` 添加了新的 api
  - `get_sender_name` 获取发送人昵称
- `ica` 兼容版本号 `2.12.11` -> `2.12.12`
- 加入了 `STABLE` 信息, 用于标记稳定版本
- 不少配置文件项加上了默认值
- 添加了 `version_str() -> String` 用于方便的获取版本信息
  - 同样在 `py` 侧也有 `version_str` 方法
- 加入了 `/help` 命令
  - 用于获取帮助信息
- 加入了 `/bot-ls`
  - 用于展示所有插件的信息
- 加入了 `/bot-enable` 和 `/bot-disable`
  - 用于启用/禁用插件

## 0.6.10

- 加了点东西 (?)

## 0.6.9

我决定立即发布 0.6.9

- 添加了 `Client.startup_time() -> datetime` 方法
  - 用于获取 bot 启动时间
  - 这样就可以经常吹我 bot 跑了多久了 ( ˘•ω•˘ )
  - 顺手加上了 `/bot-uptime` 命令
    - 可以获取 bot 运行时间
    - 谢谢 GitHub Copilot 的帮助

## 0.6.8

- 修复了一堆拼写错误
- 太难绷了
  - `TailchatReciveMessagePy` -> `TailchatReceiveMessagePy`
  - `ReciveMessage` -> `ReceiveMessage`
- `ReceiveMessage::meta`
  - 从 `JsonValue` 改成 `Option<JsonValue>`
  - 用来解决发图片的时候没有 `meta` 字段的问题
- 去除了自带的两个 macro
  - `wrap_callback` 和 `wrap_any_callback`
  - 因为现在他俩已经进到 `rust_socketio` 里啦
- 添加了新的 macro
- 支持了 `TailchatReceiveMessagePy` 的 `is_from_self` 方法
  - 用于判断是否是自己发的消息

## 0.6.7

游学回来啦

- 处理了一些 tailchat 的特殊情况
  - 比如 message 里面的 `GroupId` 实际上是可选的, 在私聊中没有这一项
  - 忽略了所有的 `__v` (用于数据库记录信息的, bot不需要管)
    - 作者原话 `不用管。数据库记录版本`
  - 修复了如果没法解析新的信息, 会 panic 的问题
- `ica_typing.py`
  - 补充了 `TailchatSendingMessage` 的 `group_id` 和 `converse_id` 字段
  - 把 `group_id` 的设置和返回都改成了 `Optional[GroupId]`
- tailchat 的 API 也差点意思就是了(逃)
- 处理了 icalingua 的 `renewMessage` 事件 (其实就是直接忽略掉了)

## 0.6.6

游学之前最后一次更新
其实也就五天

正式支持了 tailchat 端
好耶！

[!note]

```text
notice_room = []
notice_start = true

admin_list = []
filter_list = []
```

的功能暂时不支持

## 0.6.5

怎么就突然 0.6.5 了
我也不造啊

- 反正支持了 tailchat 的信息接受
- 但是需要你在对面服务端打开 `DISABLE_MESSAGEPACK` 环境变量
- 能用就行

- 现在 `update_online_data` 不会再以 INFO 级别显示了
- `update_all_room` 同上

## 0.6.2

- 添加 API
  - `NewMessage.set_img` 用于设置消息的图片
  - `IcaSendMessage.set_img` 用于设置消息的图片 (python)

## 0.6.1

还是没写完 tailchat 支持
因为 rust_socketio 还是没写好 serdelizer 的支持

- 正在添加发送图片的 api

## 0.6.0-dev

- 去除了 matrix 的支持
  - 淦哦
  - 去除了相应代码和依赖
  - 去除了 Python 侧代码
  - 向 tailchat (typescript 低头)

- 修复了没法编译的问题（

## 0.5.3

修复了 Icalingua 断开时 如果 socketio 已经断开会导致程序 返回 Error 的问题
以及还有一些别的修复就是了

- Python 端修改
  - `on_message` -> `on_ica_message`
  - `on_delete_message` -> `on_ica_delete_message`
  - 添加 `on_matrix_message`

## 0.5.1/2

重构了一整波, 还没改 `ica-typing.py` 的代码
但至少能用了

- Ica 版本号 `1.4.0`
- Matrix 版本号 `0.1.0`

## 0.5.0

准备接入 `Matrix`

去掉 `pyo3-async` 的依赖

## 0.4.12

把 0.4.11 的遗留问题修完了

## 0.4.11

这几天就是在刷版本号的感觉

- 添加
  - `DeleteMessage` 用于删除消息
  - `NewMessage.as_delete` 用于将消息转换为删除消息
  - `client::delete_message` 用于删除消息
  - `client::fetch_history` 用于获取历史消息 TODO
  - `py::class::DeleteMessagePy` 用于删除消息 的 Python 侧 API
  - `py::class::IcaClientPy.delete_message` 用于删除消息 的 Python 侧 API
  - `IcalinguaStatus.current_loaded_messages_count`
    - 用于以后加载信息计数
- 修改
  - `py::class::IcaStatusPy`
    - 大部分方法从手动 `unsafe` + `Option`
    - 改成直接调用 `IcalinguaStatus` 的方法
  - `IcalinguaStatus`
    - 所有方法均改成 直接对着 `IcalinguaStatus` 的方法调用
    - 补全没有的方法

## 0.4.10

好家伙, 我感觉都快能叫 0.5 了
修改了一些内部数据结构, 使得插件更加稳定

添加了 `rustfmt.toml` 用于格式化代码
**注意**: 请在提交代码前使用 `cargo +nightly fmt` 格式化代码

修复了 `Message` 解析 `replyMessage` 字段是 如果是 null 则会解析失败的问题

## 0.4.9

修复了 Python 插件运行错误会导致整个程序崩溃的问题

## 0.4.8

添加了 `filter_list` 用于过滤特定人的消息

## 0.4.7

修复了重载时如果代码有问题会直接 panic 的问题

## 0.4.6

现在更适合部署了

## 0.4.5

添加 `is_reply` api 到 `NewMessagePy`

## 0.4.4

现在正式支持 Python 插件了
`/bmcl` 也迁移到了 Python 插件版本

## 0.4.3

噫! 好! 我成了!

## 0.4.2

现在是 async 版本啦!

## 0.4.1

现在能发送登录信息啦

## 0.4.0

使用 Rust 从头实现一遍
\能登录啦/

## 0.3.3

适配 Rust 端的配置文件修改

## 0.3.1/2

改进 `/bmcl` 的细节

## 0.3.0

合并了 dongdigua 的代码, 把消息处理部分分离
现在代码更阳间了（喜

## 0.2.3

添加了 `/bmcl` 请求 bmclapi 状态

## 0.2.2

重构了一波整体代码
