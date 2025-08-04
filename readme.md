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
