//! Python 插件与 Bridge 的 JSON 边界；在持有解释器时转换，在释放 GIL 后只传拥有的数据。

use pyo3::{
    Bound, Py, PyAny, PyResult, Python,
    exceptions::PyValueError,
    types::{PyAnyMethods, PyDict, PyDictMethods},
};
use serde_json::Value;

/// 使用 Python 自带 JSON 编码处理容器；拒绝非 JSON 类型与非有限数字。
pub fn py_to_json(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let py = value.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("allow_nan", false)?;
    let encoded: String =
        py.import("json")?.getattr("dumps")?.call((value,), Some(&kwargs))?.extract()?;
    serde_json::from_str(&encoded).map_err(|error| PyValueError::new_err(error.to_string()))
}

/// 返回独立 Python JSON 快照，插件对它的修改不会污染 Bot 缓存。
pub fn json_to_py(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    let encoded =
        serde_json::to_string(value).map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(py.import("json")?.getattr("loads")?.call1((encoded,))?.unbind())
}
