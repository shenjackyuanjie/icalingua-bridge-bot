# icalingua bot

这是一个基于 icalingua docker 版的 bot

> 出于某个企鹅, 和保护 作者 和 原项目 ( ica ) 的原因 \
> 功能请自行理解

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
# 用你自己的方法启动你的 icalingua 后端
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
