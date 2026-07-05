//! 暴露给 Python 插件的定时任务控制类型。

use std::time::Duration;

use pyo3::{Bound, Py, PyTraverseError, PyVisit, Python, pyclass, pymethods, types::PyFunction};
use tracing::{Level, event};

#[derive(Debug)]
#[pyclass]
#[pyo3(name = "Scheduler")]
/// 用于计划任务的类
///
/// 给 Python 侧使用
///
/// add: 0.9.0
pub struct SchedulerPy {
    /// 回调函数
    ///
    /// 你最好不要把他清理掉
    pub callback: Py<PyFunction>,
    /// 预计等待时间
    pub schdule_time: Duration,
}

#[pymethods]
impl SchedulerPy {
    /// 遍历 Python 对象持有的引用。
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.callback)?;
        Ok(())
    }

    #[new]
    /// 创建并初始化对应的数据结构。
    pub fn new(func: Bound<'_, PyFunction>, schdule_time: Duration) -> Self {
        Self {
            callback: func.unbind(),
            schdule_time,
        }
    }

    /// 开始
    pub fn start(&self, py: Python<'_>) {
        let wait = self.schdule_time;
        let cb = self.callback.clone_ref(py);
        tokio::spawn(async move {
            let second = Duration::from_secs(1);
            if wait > second {
                let big_sleep = wait.checked_sub(second).unwrap();
                tokio::time::sleep(big_sleep).await;
                tokio::time::sleep(second).await;
            } else {
                tokio::time::sleep(wait).await;
            }
            Python::attach(|py| {
                event!(Level::INFO, "正在调用计划 {:?}", wait);
                if let Err(e) = cb.call0(py) {
                    event!(Level::WARN, "调用时出现错误 {}", e);
                }
            });
        });
    }
}
