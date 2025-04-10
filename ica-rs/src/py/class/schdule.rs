use pyo3::{Bound, Py, PyAny, PyTraverseError, PyVisit, pyclass, pymethods, types::PyFunction};
use tracing::{Level, event};

#[derive(Clone, Debug)]
#[pyclass]
#[pyo3(name = "Scheduler")]
/// 用于计划任务的类
///
/// 给 Python 侧使用
pub struct SchedulerPy {
    /// 回调函数
    ///
    /// 你最好不要把他清理掉
    pub callback: Py<PyFunction>,
}

#[pymethods]
impl SchedulerPy {
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.callback)?;
        Ok(())
    }
}
