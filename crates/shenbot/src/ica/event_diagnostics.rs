//! 无效 Bridge 事件的实例级日志限频，不保留消息内容或标识符。

#[cfg(test)]
#[path = "tests/event_diagnostics.rs"]
mod tests;

use std::collections::HashMap;
use std::time::{Duration, Instant};

const REPORT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct EventWindow {
    reported_at: Instant,
    rejected: u64,
}

#[derive(Debug, Clone, Default)]
pub struct EventDiagnostics {
    // 只由代码中固定的事件名称索引，不把 Bridge 输入当作键。
    windows: HashMap<&'static str, EventWindow>,
}

impl EventDiagnostics {
    /// 第一次立即提示；后续每 30 秒至多一次，返回自上次提示以来的拒收数量。
    pub fn record_invalid(&mut self, event: &'static str, now: Instant) -> Option<u64> {
        match self.windows.get_mut(event) {
            None => {
                self.windows.insert(
                    event,
                    EventWindow {
                        reported_at: now,
                        rejected: 0,
                    },
                );
                Some(1)
            }
            Some(window) => {
                window.rejected = window.rejected.saturating_add(1);
                if now.saturating_duration_since(window.reported_at) < REPORT_INTERVAL {
                    return None;
                }
                let rejected = window.rejected;
                window.reported_at = now;
                window.rejected = 0;
                Some(rejected)
            }
        }
    }
}
