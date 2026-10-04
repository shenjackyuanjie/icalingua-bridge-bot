# ica-bot 协作指南

本目录是独立的 Rust workspace，遵循上层工作区指南。

## 协议跟进范围

- 跟进 Icalingua-plus-plus 本体或参考 ica-native 的协议、功能补全时，**只针对 Socket.IO + oicq Bridge**。
- **不跟进 Milky 或 OneBot**，不为其新增兼容分支、字段推断、接口适配或专门测试，除非用户明确另行要求。
- 不因这项范围约定擅自删除仓库已有的其他 adapter 代码；保持原有代码，不扩展适配。
- 区分本体发布版本和 Bridge 的 `protocolVersion`；以本地 oicq Bridge 的类型、事件注册和实际参数契约为准。

## 验证与文档

- Rust 改动运行 `cargo +nightly fmt --all --check`、`cargo test --workspace` 和 `cargo clippy --workspace --all-targets`。
- 用户可见的 API 或兼容行为变更同步记录到 `news/0-9.md`；Python 插件 API 变更同步检查独立 `ica-plugin` 仓库的类型声明，不混合两仓库提交。
