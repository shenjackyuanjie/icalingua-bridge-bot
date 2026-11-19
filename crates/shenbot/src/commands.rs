//! 与聊天协议无关的内置命令解析、权限和命令执行。
use crate::{
    VERSION,
    host::{BotHost, PluginAction, PluginChange},
    version_for,
};
use std::{future::Future, sync::Arc};

const HELP: &str = r#"/bot-rs
    展示 rust 侧信息
/bot-py
    展示 python 侧信息(如果python插件启用了的话)
/bot-ls
    显示所有插件信息
/bot-permission
    显示自己的权限组（ICA）
/bot-enable-<client-id> <plugin>
    启用某个插件(具体到客户端)
/bot-disable-<client-id> <plugin>
    禁用某个插件(具体到客户端)
/bot-reload-<client-id> <plugin>
    重载某个插件(具体到客户端，仅 ICA)
/bot-fetch
    更新当前房间消息（ICA，管理员）

by shenjackyuanjie"#;

pub(crate) fn help(client_id: &str) -> String {
    format!(
        "{}
{}",
        version_for(client_id),
        HELP.replace("<client-id>", client_id)
    )
}

#[derive(Clone, Copy)]
pub(crate) enum Source {
    Ica { from_self: bool },
    Tailchat,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Command {
    RustVersion,
    PythonVersion,
    List,
    Help,
    Permission { admin: bool },
    Manage(PluginAction, String),
    FetchRoom,
}

pub(crate) fn parse(
    content: &str,
    source: Source,
    client_id: &str,
    is_reply: bool,
    is_admin: bool,
) -> Option<Command> {
    if is_reply || matches!(source, Source::Ica { from_self: true }) {
        return None;
    }
    let ica = matches!(source, Source::Ica { .. });
    Some(match content {
        "/bot-rs" => Command::RustVersion,
        "/bot-py" => Command::PythonVersion,
        "/bot-ls" => Command::List,
        "/bot-help" => Command::Help,
        "/bot-permission" if ica => Command::Permission { admin: is_admin },
        "/bot-fetch" if ica && is_admin => Command::FetchRoom,
        _ if is_admin => {
            let (verb, id) = content.split_once(' ')?;
            let (action, target) = verb.strip_prefix("/bot-")?.rsplit_once('-')?;
            if target != client_id {
                return None;
            }
            let action = match action {
                "enable" => PluginAction::Enable,
                "disable" => PluginAction::Disable,
                "reload" if ica => PluginAction::Reload,
                _ => return None,
            };
            Command::Manage(action, id.to_owned())
        }
        _ => return None,
    })
}

pub(crate) enum Reply {
    Text(String),
    FetchRoom,
}

/// 慢生命周期事务不能在 Socket.IO 回调中等待：读循环还要消费在途动作的回执。
/// complete 同时投递该消息的普通 hook，保留本条命令在其 hook 之前完成的语义。
pub(crate) async fn dispatch<F, Fut>(host: &Arc<BotHost>, command: Option<Command>, complete: F)
where
    F: FnOnce(Option<Reply>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let Some(command) = command.filter(|_| host.is_accepting()) else {
        complete(None).await;
        return;
    };
    if let Command::Manage(action, id) = command {
        let Ok(permit) = host.command_slots.clone().try_acquire_owned() else {
            complete(Some(Reply::Text("管理命令繁忙，请稍后再试".into()))).await;
            return;
        };
        let host = host.clone();
        host.context.runtime.clone().spawn(async move {
            let _permit = permit;
            let text = manage_reply(action, host.change_plugin(&id, action).await);
            complete(Some(Reply::Text(text))).await;
        });
        return;
    }
    let reply = match command {
        Command::RustVersion => Some(Reply::Text(host.context.version_str())),
        Command::PythonVersion => {
            host.python_version().map(|version| Reply::Text(format!("Python {version}")))
        }
        Command::List => Some(Reply::Text(plugin_list(host).await)),
        Command::Help => Some(Reply::Text(help(host.context.client_id()))),
        Command::Permission { admin } => {
            Some(Reply::Text(format!("您的权限: {}", if admin { "管理员" } else { "没啥" })))
        }
        Command::FetchRoom => Some(Reply::FetchRoom),
        Command::Manage(..) => unreachable!("管理命令已移交后台任务"),
    };
    complete(reply).await;
}

async fn plugin_list(host: &BotHost) -> String {
    let plugins = host.plugins().await;
    let enabled = plugins.iter().filter(|entry| entry.enabled).count();
    let mut list = format!(
        "shenbot v{VERSION}-{}
插件列表 ({enabled} / {}):",
        host.context.client_id(),
        plugins.len()
    );
    if plugins.is_empty() && !host.context.config.check_py() {
        list.push_str(
            "
未启用 Python 插件",
        );
    }
    for entry in plugins {
        use std::fmt::Write;
        let disabled = if entry.enabled { "" } else { " [禁用]" };
        let _ = write!(
            list,
            "
[{}] {}({}){disabled}",
            entry.kind.label(),
            entry.id,
            entry.name
        );
    }
    list
}

fn manage_reply(action: PluginAction, result: PluginChange) -> String {
    use PluginAction::{Disable, Enable, Reload};
    use PluginChange::{Changed, Failed, NotFound, Unchanged};
    match (action, result) {
        (_, NotFound) => "未找到插件".into(),
        (Enable, Unchanged) => "无变化, 插件已经启用".into(),
        (Disable, Unchanged) => "无变化, 插件已经禁用".into(),
        (Enable, Changed) => "启用插件完成".into(),
        (Disable, Changed) => "禁用插件完成".into(),
        (Reload, Changed | Unchanged) => "重载成功".into(),
        (Enable, Failed(error)) => format!("启用插件失败, 错误: \n{error}"),
        (Disable, Failed(error)) => format!("禁用插件失败, 错误: \n{error}"),
        (Reload, Failed(error)) => format!("重载失败, 错误: \n{error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_requires_admin_and_an_exact_client_id() {
        let ica = Source::Ica { from_self: false };
        let disable = "/bot-disable-123456 demo";
        assert_eq!(
            parse(disable, ica, "123456", false, true),
            Some(Command::Manage(PluginAction::Disable, "demo".into()))
        );
        assert_eq!(parse(disable, ica, "123456", false, false), None);
        assert_eq!(parse(disable, ica, "654321", false, true), None);
        assert_eq!(parse("/bot-disable-123456-extra demo", ica, "123456", false, true), None);
        assert_eq!(parse(disable, ica, "123456", true, true), None);
        assert_eq!(parse(disable, Source::Ica { from_self: true }, "123456", false, true), None);
    }

    #[test]
    fn ica_only_commands_are_not_silently_enabled_in_tailchat() {
        let ica = Source::Ica { from_self: false };
        assert_eq!(parse("/bot-fetch", ica, "123456", false, false), None);
        assert_eq!(parse("/bot-fetch", ica, "123456", false, true), Some(Command::FetchRoom));
        for content in ["/bot-fetch", "/bot-permission", "/bot-reload-123456 demo"] {
            assert_eq!(parse(content, Source::Tailchat, "123456", false, true), None);
        }
    }
}
