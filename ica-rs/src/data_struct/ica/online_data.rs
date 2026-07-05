//! Icalingua bridge 在线状态和服务端运行信息。

use serde_json::Value as JsonValue;
use tracing::warn;

#[derive(Debug, Clone, Hash)]
pub struct IcalinguaInfo {
    pub ica_version: String,
    pub os_info: String,
    pub resident_set_size: String,
    pub heap_used: String,
    pub load: String,
    /// 服务器 nodejs 版本
    pub server_node: String,
    /// 客户端计数
    /// 我还不新 u16 (u16::MAX) 会溢出
    pub client_count: u16,
}

impl IcalinguaInfo {
    /// 创建并初始化对应的数据结构。
    pub fn new_from_str(s: &str) -> IcalinguaInfo {
        let mut ica_version = None;
        let mut os_info = None;
        let mut resident_set_size = None;
        let mut heap_used = None;
        let mut load = None;
        let mut server_node = None;
        let mut client_count = None;
        let info_list = s.split('\n').collect::<Vec<&str>>();
        for info in info_list {
            if info.starts_with("icalingua-bridge-oicq") {
                ica_version = Some(info.split_at(22).1.to_string());
            } else if info.starts_with("Running on") {
                os_info = Some(info.split_at(11).1.to_string());
            } else if info.starts_with("Resident Set Size") {
                resident_set_size = Some(info.split_at(18).1.to_string());
            } else if info.starts_with("Heap used") {
                heap_used = Some(info.split_at(10).1.to_string());
            } else if info.starts_with("Load") {
                load = Some(info.split_at(5).1.to_string());
            } else if info.starts_with("Server Node") {
                server_node = Some(info.split_at(12).1.to_string());
            } else if info.ends_with("clients connected") {
                client_count = Some(
                    info.split(' ')
                        .collect::<Vec<&str>>()
                        .first()
                        .unwrap_or(&"1")
                        .parse::<u16>()
                        .unwrap_or_else(|e| {
                            warn!("client_count parse error: {}|raw: {}", e, info);
                            1_u16
                        }),
                );
            }
        }
        IcalinguaInfo {
            ica_version: ica_version.unwrap_or_else(|| {
                warn!("ica_version faild to parse");
                "UNKNOWN".to_string()
            }),
            os_info: os_info.unwrap_or_else(|| {
                warn!("os_info faild to parse");
                "UNKNOWN".to_string()
            }),
            resident_set_size: resident_set_size.unwrap_or_else(|| {
                warn!("resident_set_size faild to parse");
                "UNKNOWN".to_string()
            }),
            heap_used: heap_used.unwrap_or_else(|| {
                warn!("heap_used faild to parse");
                "UNKNOWN".to_string()
            }),
            load: load.unwrap_or_else(|| {
                warn!("load faild to parse");
                "UNKNOWN".to_string()
            }),
            server_node: server_node.unwrap_or_else(|| {
                warn!("server_node faild to parse");
                "UNKNOWN".to_string()
            }),
            client_count: client_count.unwrap_or_else(|| {
                warn!("client_count faild to parse");
                1_u16
            }),
        }
    }
}

/// {"bkn":<bkn>,
/// "nick":<nick>,
/// "online":true,
/// "sysInfo":"icalingua-bridge-oicq 2.11.1\n
/// Running on Linux c038fad79f13 4.4.302+\n
/// Resident Set Size 95.43MB\n
/// Heap used 37.31MB\n
/// Load 4.23 2.15 1.59\n
/// Server Node 18.19.0\n
/// 2 clients connected",
/// "uin": <qqid> }
#[derive(Debug, Clone, Hash)]
pub struct OnlineData {
    pub bkn: i64,
    pub nick: String,
    pub online: bool,
    pub qqid: i64,
    pub icalingua_info: IcalinguaInfo,
}

impl OnlineData {
    /// 创建并初始化对应的数据结构。
    pub fn new_from_json(value: &JsonValue) -> OnlineData {
        let bkn = value["bkn"].as_i64().unwrap_or_else(|| {
            warn!("bkn not found in online data");
            -1
        });
        let nick = value["nick"]
            .as_str()
            .unwrap_or_else(|| {
                warn!("nick not found in online data");
                "UNKNOWN"
            })
            .to_string();
        let online = value["online"].as_bool().unwrap_or_else(|| {
            warn!("online not found in online data");
            false
        });
        let qqid = value["uin"].as_i64().unwrap_or_else(|| {
            warn!("uin not found in online data");
            -1
        });
        let sys_info = value["sysInfo"].as_str().unwrap_or_else(|| {
            warn!("sysInfo not found in online data");
            ""
        });
        let icalingua_info = IcalinguaInfo::new_from_str(sys_info);
        OnlineData {
            bkn,
            nick,
            online,
            qqid,
            icalingua_info,
        }
    }
}

impl Default for OnlineData {
    /// 构造当前类型的默认值。
    fn default() -> Self {
        OnlineData {
            bkn: -1,
            nick: "UNKNOWN".to_string(),
            online: false,
            qqid: -1,
            icalingua_info: IcalinguaInfo {
                ica_version: "UNKNOWN".to_string(),
                os_info: "UNKNOWN".to_string(),
                resident_set_size: "UNKNOWN".to_string(),
                heap_used: "UNKNOWN".to_string(),
                load: "UNKNOWN".to_string(),
                server_node: "UNKNOWN".to_string(),
                client_count: 1,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 解析 `empty_str` 数据。
    fn parse_empty_str() {
        let icalingua_info = IcalinguaInfo::new_from_str("");
        assert_eq!(icalingua_info.ica_version, "UNKNOWN");
        assert_eq!(icalingua_info.os_info, "UNKNOWN");
        assert_eq!(icalingua_info.resident_set_size, "UNKNOWN");
        assert_eq!(icalingua_info.heap_used, "UNKNOWN");
        assert_eq!(icalingua_info.load, "UNKNOWN");
        assert_eq!(icalingua_info.server_node, "UNKNOWN");
        assert_eq!(icalingua_info.client_count, 1);
    }

    #[test]
    /// 解析 `str` 数据。
    fn parse_str() {
        let icalingua_info = IcalinguaInfo::new_from_str(
            "icalingua-bridge-oicq 2.11.1\n\
            Running on Linux c038fad79f13 4.4.302+\n\
            Resident Set Size 95.43MB\n\
            Heap used 37.31MB\n\
            Load 4.23 2.15 1.59\n\
            Server Node 18.19.0\n\
            2 clients connected",
        );
        assert_eq!(icalingua_info.ica_version, "2.11.1");
        assert_eq!(icalingua_info.os_info, "Linux c038fad79f13 4.4.302+");
        assert_eq!(icalingua_info.resident_set_size, "95.43MB");
        assert_eq!(icalingua_info.heap_used, "37.31MB");
        assert_eq!(icalingua_info.load, "4.23 2.15 1.59");
        assert_eq!(icalingua_info.server_node, "18.19.0");
        assert_eq!(icalingua_info.client_count, 2);
    }

    #[test]
    /// 解析 `online_data` 数据。
    fn parse_online_data() {
        let online_data = OnlineData::new_from_json(&serde_json::json!({
            "bkn": 123,
            "nick": "test",
            "online": true,
            "sysInfo": "icalingua-bridge-oicq 2.11.1\n\
            Running on Linux c038fad79f13 4.4.302+\n\
            Resident Set Size 95.43MB\n\
            Heap used 37.31MB\n\
            Load 4.23 2.15 1.59\n\
            Server Node 18.19.0\n\
            2 clients connected",
            "uin": 123456
        }));
        assert_eq!(online_data.bkn, 123);
        assert_eq!(online_data.nick, "test");
        assert!(online_data.online);
        assert_eq!(online_data.qqid, 123456);
        assert_eq!(online_data.icalingua_info.ica_version, "2.11.1");
        assert_eq!(online_data.icalingua_info.os_info, "Linux c038fad79f13 4.4.302+");
        assert_eq!(online_data.icalingua_info.resident_set_size, "95.43MB");
        assert_eq!(online_data.icalingua_info.heap_used, "37.31MB");
        assert_eq!(online_data.icalingua_info.load, "4.23 2.15 1.59");
        assert_eq!(online_data.icalingua_info.server_node, "18.19.0");
        assert_eq!(online_data.icalingua_info.client_count, 2);
    }

    #[test]
    /// 解析 `online_data_empty` 数据。
    fn parse_online_data_empty() {
        let online_data = OnlineData::new_from_json(&serde_json::json!({}));
        assert_eq!(online_data.bkn, -1);
        assert_eq!(online_data.nick, "UNKNOWN");
        assert!(!online_data.online);
        assert_eq!(online_data.qqid, -1);
        assert_eq!(online_data.icalingua_info.ica_version, "UNKNOWN");
        assert_eq!(online_data.icalingua_info.os_info, "UNKNOWN");
        assert_eq!(online_data.icalingua_info.resident_set_size, "UNKNOWN");
        assert_eq!(online_data.icalingua_info.heap_used, "UNKNOWN");
        assert_eq!(online_data.icalingua_info.load, "UNKNOWN");
        assert_eq!(online_data.icalingua_info.server_node, "UNKNOWN");
        assert_eq!(online_data.icalingua_info.client_count, 1);
    }
}
