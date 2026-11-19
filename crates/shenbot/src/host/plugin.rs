//! 中立插件视图和当前宿主实例的全局 ID 所有权。
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginKind {
    Py,
    Ext,
}

impl PluginKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Py => "py",
            Self::Ext => "ext",
        }
    }
}

#[derive(Debug)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
    pub enabled: bool,
}

#[derive(Debug, Default)]
pub struct PluginRegistry {
    owners: Mutex<HashMap<String, PluginKind>>,
}

impl PluginRegistry {
    /// 同类和跨类均不得重复注册；停用或重载不释放原占位。
    pub fn claim(&self, id: &str, kind: PluginKind) -> Result<(), PluginKind> {
        use std::collections::hash_map::Entry;
        match self.owners.lock().unwrap().entry(id.to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(kind);
                Ok(())
            }
            Entry::Occupied(entry) => Err(*entry.get()),
        }
    }

    pub fn owner(&self, id: &str) -> Option<PluginKind> {
        self.owners.lock().unwrap().get(id).copied()
    }

    /// 插件已经完成排空、从后端移除并退役后才可释放 ID。
    pub fn release(&self, id: &str, kind: PluginKind) {
        let mut owners = self.owners.lock().unwrap();
        if owners.get(id) == Some(&kind) {
            owners.remove(id);
        }
    }
}
