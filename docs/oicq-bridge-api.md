# ICA 插件 API 2.0.6：Socket.IO + oicq

## 实现基准

本轮以 Icalingua++ 独立 oicq Bridge 的事件注册、位置参数和实际 ACK 为基准，参考协议版本为 **2.26.7**；Bot 发布版本为 **0.9.5**，ICA Python 模块/API 为 **2.0.6**。独立 Bridge 的上传缓冲补丁服务版本为 **2.26.9**，Electron 版本保持不变。服务发布版本、协议版本与插件 API 版本分别管理，Bot 版本记录见 [更新日志](../news/0-9.md#095)。

ica-native 用作功能覆盖参考。本轮提供 Bot 可消费的消息、查询、转发、文件与运维事件，不移植桌面窗口，也不扩展 Milky/OneBot、群公告 HTTP 业务或同进程多 Bridge。

正式 Python 签名和实际管理员插件见独立插件仓库的 [API 指南](../../ica-plugin/docs/oicq-plugin-api.md)、[类型声明](../../ica-plugin/ica_typing.py) 与 [bridge_query 插件](../../ica-plugin/plugins/bridge_query.py)。

## 结果语义与通用调用

- `send_message`、`send_raw_message`、`send_file`、`send_merged_forward` 和 `emit_event` 的 `True` 只表示传输受理，不是 QQ 最终发送成功，也不是消息 ID 回执。
- `request_event(event, args=None, timeout=15.0)` 等待真实 Socket.IO ACK，返回事件原有的 JSON 结果。ACK 不一定表示业务成功，仍需检查该事件的真实返回形状。
- `args` 为位置参数列表。可选参数保留 `None` 占位，不把所有参数包进一个对象。
- 已知无 ACK 修改事件应使用 `emit_event`；主连接上的 fileMgr 事件和当前未实现事件会明确报错。未知扩展事件保持可调用，请求有本地截止时间。
- `timeout` 单位为秒，范围为 `(0, 300]`。参数错误为 `ValueError`，请求失败为 `RuntimeError`，与合法空页 `[]` 分开处理。

例如，在用户明确操作的分支中查询资料：

```python
member = client.request_event("getGroupMemberInfo", [group_id, user_id, False])
accepted = client.emit_event("setFriendRemark", [user_id, remark])
# accepted 不证明 QQ 已成功修改备注。
```

`messageSuccess/messageError/notifyError/addMessageText` 等提示可能广播到多个客户端，缺少统一请求 ID，不能拿最近一次提示当作某次发送的确定结果。

## 消息结构与更新

旧 `id/content/sender_id/sender_name/room_id/reply_msg_*` 接口保持。新增可消费的附件、时间、接收提及标记、卡片、引用、消息状态、按钮和撤回资料。

- `files` 是结构化附件列表；64 位文件大小、尺寸、顺序及未知扩展字段保留。兼容旧单 `file`，镜像附件不重复。历史/引用文件尚未取得下载地址时，`url` 为 `""`，可继续按 `fid` 查询，不应直接当成可下载 URL。
- `time` 有有效原消息时间时为 Unix **毫秒**整数，缺失或超出可表示范围时为 `None`；`0` 是有效时间。
- 接收 `at` 为 `bool | "all" | None`；发送 `IcaSendMessage.at` 是 `[{"id": user_id, "text": nickname}, {"id": "all"}]`，两者含义不同。
- `raw_data` 保留收到的完整 Bridge Message 输入（包括降级字段的原值）；`to_dict()` 输出当前模型的规范化快照。`raw_msg` 仅代表可选原协议节点链，没有链时为 `None`。
- 字符串/数字消息 ID 统一成字符串。消息对象、ID、正文/昵称显式错误类型及无效来源房间返回解析错误；其他可选字段按字段降级，附件逐项保留合法项，坏引用不阻断正文。未知接收 @ 标记为 `None`，不冒充全体提及。
- 更新补丁继续严格验证并原子应用，错误时保留缓存；接收侧的容错不代表允许坏补丁清空有效信息。无效事件首次警告，同一事件后续每 30 秒最多提示一次，并汇总窗口内拒收数量。

新增两个 client-first hook，原有 hook 顺序不变：

```python
def on_ica_message_updated(client, room_id, message_id, patch):
    # patch 是增量字段；可从 get_loaded_messages 查询已缓存上下文。
    pass


def on_ica_bridge_event(client, event, args):
    # 用于状态、登录挑战、操作提示及扩展推送。
    # 不原样记录 Cookie、二维码、验证 URL 或整个消息 payload。
    pass
```

`renewMessage` 的 QQ 撤回映射到原有删除 hook，并有界去重；媒体后处理和 URL 更新通过更新 hook/原事件交给插件。历史回补不重放 `on_ica_message` 或系统消息 hook。

## 查询与缓存

```python
get_loaded_messages(room_id, limit=200)
fetch_messages(room_id, options=None, timeout=15.0)
search_messages(room_id, keyword, offset=0, sender_id=None,
                start_time=None, end_time=None, timeout=15.0)
fetch_messages_by_sender(room_id, sender_id, offset=0,
                         snapshot_time=None, timeout=15.0)
fetch_messages_around(room_id, message_id, before=20, after=20, timeout=15.0)
fetch_image_messages(room_id, offset=0, end_time=None, timeout=15.0)
get_system_messages(timeout=15.0)
get_friends(timeout=15.0)
get_groups(timeout=15.0)
```

消息查询返回 `list[IcaNewMessage]`，可供摘要、检索、媒体处理及构造回复。查询先完整验证返回数据，再按真实房间合并入有界缓存；跨房间 `search_messages(0, ...)` 必须有各项来源 `roomId`。转发资源不混入房间历史。

缓存默认每房间 200 条、最多 64 房间、消息 JSON 估算负载合计 16 MiB，并按 LRU/最旧消息淘汰。它是已加载集合，不是 Bridge 全量数据库。`setMessages` 的首屏回补合并而非覆盖整个已加载历史；`/bot-fetch` 会消费 ACK 并缓存，不再只打印条数。

`room_id > 0` 是私聊，`room_id < 0` 是负群号。普通/搜索/发言人查询固定每页 20 条，图片页 30 条，没有统一 `total/hasMore` 对象。游标形如 `{"before": {"time": milliseconds, "id": message_id}}`，`before/after` 互斥。搜索边界、发送者快照与图片上界同样使用毫秒。

**首屏 `fetch_messages` 有副作用**：Bridge 会清房间未读/提及状态，并内部上报最近消息 QQ 已读。其他本地查询不会自动获取全部 QQ 漫游历史。

实际管理员插件提供 `/bot-history [1..20]`、`/bot-search <关键词>` 和回复 `/bot-media`。只查询当前房间，输出一条限长、协议标记转纯文本的摘要；不下载媒体、不扫描其他房间、不逐条回放历史。

## 原始节点与合并转发

```python
new_message(room_id, content="", reply=None)
send_raw_message(raw_msg, room_id)
get_forward_messages(res_id, file_name=None, timeout=15.0)
send_merged_forward(nodes, target, dm=None, origin=None)
```

`send_raw_message` 接受含非空 `type` 的非空节点 **JSON 数组字符串**。实际发送采用 `messageType="raw"`、JSON 字符串 `content` 与 `at=[]`；非法 JSON/单对象/空链在发送前拒绝。它不是任意 Socket.IO API。

`new_message` 只构造不自动发送，可用查询消息的 `to_dict()` 作回复快照。`to_forward_node()` 优先原节点链，否则重建可用文字/图片/语音资源，对无法重建的普通文件明确降级成说明，不冒造 QQ 文件资源。

`send_merged_forward` 接收本体 FakeMessage 节点，不是规范化 Message 列表。推荐 `[record.to_forward_node() for record in records]`。FakeMessage 的 `time` 是 Unix **秒**，与查询 Message 的毫秒不同；`origin` 为整数或 `None`。转发资源展开结果默认房间为 0，不写入普通会话缓存。

## 上传与 fileMgr

```python
upload_file(file_name, data)  # data 为显式读取的 bytes，返回已验证临时 hash
send_file(room_id, file_name, data, content="", mime="application/octet-stream",
          reply=None, at=None)
upload_group_file(group_id, file_name, data, parent="/")
file_manager_request(group_id, event, args=None, timeout=15.0)
```

上传沿用本体固定 **512 KiB** 分片，SHA-256 校验、缺片续传及最终整文件校验。只有幂等分片至多重试三次；不确定的 QQ 发送/群文件提交不自动重试。临时 hash 在后续提交后可能清理。

`send_file` 不接受图片/音频 MIME，应使用既有媒体接口；需要作为文档发送时显式传 `application/octet-stream`。本体 hash 文件分支提前返回，**`reply/@` 不加入 QQ 文件链**，`content` 最多参与私聊回显，不能理解为文件同时附带正文/引用/@。

群文件 API 接受正群号。fileMgr 先在主连接获取群 token，再建立专用 `auth(token, "fileMgr")` 短连接，独立处理建连、认证、ACK 超时和清理。七个真实事件为 `ls/download/stat/mkdir/mv/rm/rename`，`mkdir` 只有名字参数。

当前 Rust Socket.IO SDK 不支持多个位置参数混合二进制附件，上传使用本体接受的 JSON 字节数组。512 KiB 最坏编码约 2 MiB；本工作区独立 Bridge 接收上限已调整为 **4 MiB**，部署时需同步。分片大小、offset、哈希与协议版本保持不变。

## reportRead 的实际情况

`reportRead(messageId)` 是 QQ 协议层的已读上报，**不是**更新 Bridge 数据库里的未读数。

1. Electron/native/Bot 发送过这个 Socket 事件。
2. oicq adapter 存在调用 `bot.reportReaded` 的实现。
3. 独立 Bridge 的主 Socket 注册表没有注册 `reportRead`，因此正常远程事件到不了该实现。
4. 首屏 `fetchMessages` 内部直接调用上报逻辑，这是仍存在的另一条路径。

旧 `report_read` 保留发送事件的兼容行为，但返回成功只表示 emit 受理。新通用接口对当前未注册的 `reportRead/stopFetchingHistory/preloadImages` 和 oicq 显式未实现的 `getPrivateFileUrl` 报错，不制造可用性。本轮未补服务端已读接线，也不把它计为 native 可用优势。

## Windows 构建与验证

使用 Rust 1.88+、Visual Studio x64 C++ 开发环境和匹配 PyO3 的 Python，推荐 Python 3.12。下面只改变当前 PowerShell 进程的环境，从 Bot 仓库根执行：

```powershell
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$dev = Join-Path $vs 'Common7\Tools\VsDevCmd.bat'
$lines = & cmd.exe /d /s /c "`"$dev`" -no_logo -arch=x64 -host_arch=x64 >nul && set"
if ($LASTEXITCODE -ne 0) { throw 'MSVC 开发环境初始化失败' }
foreach ($line in $lines) {
    if ($line -match '^([^=]+)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
$env:PYO3_PYTHON = (Get-Command python).Source  # 选定的 Python 环境
$tmp = Join-Path $PWD 'target\build-temp'
New-Item -ItemType Directory -Force $tmp | Out-Null
$env:TEMP = $tmp
$env:TMP = $tmp
cargo build --release --locked
```

如果 `aws-lc-sys` 最后由 `lib.exe` 报 `LNK1104`，以实际打不开的路径判定问题；MSVC/NASM 功能探测告警不是最终归档失败的证据。项目内临时目录方案用于避免当前用户 Temp 的构建访问问题，不更改系统环境或 ACL，也不需要回退已兼容更新的依赖。

如果 Rust 已完成依赖编译，但最后无法删除或覆盖 `target/release/ica-rs.exe`，先确认该文件是否由运行中的 Bot 占用。这与 C 依赖构建失败是两类问题。需要保持当前 Bot 运行时，可以沿用上面的开发环境和临时目录设置，将新产物构建到独立目录：

```powershell
cargo build --release --locked --target-dir target/validation-build
# 新产物：target/validation-build/release/ica-rs.exe
```

此操作不停止旧 Bot，也不替换默认产物；实际部署时再由操作者安排停机与替换。

业务验证从各独立仓库执行：

```powershell
# ica-bot
cargo +nightly fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked

# ica-plugin
uv run --no-project --python 3.12 -m unittest discover -s tests -v
```

离线业务测试覆盖消息解析/更新、缓存边界、查询归属、raw 与转发合同、ACK 和文件续传、插件权限与单条摘要。构建及离线测试不替代 QQ 在线权限、Cookie/远端资源有效性或最终消息送达验证。
