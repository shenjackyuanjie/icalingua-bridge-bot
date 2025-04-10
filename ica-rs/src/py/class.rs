pub mod config;
pub mod ica;
pub mod schdule;
pub mod tailchat;

use pyo3::{
    pyclass, pymethods, pymodule,
    types::{PyBool, PyModule, PyModuleMethods, PyString},
    Bound, IntoPyObject, PyAny, PyRef, PyResult,
};
use toml::Value as TomlValue;
use tracing::{event, Level};

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "ConfigData")]
pub struct ConfigDataPy {
    pub data: TomlValue,
}

#[pymethods]
impl ConfigDataPy {
    pub fn __getitem__(self_: PyRef<'_, Self>, key: String) -> Option<Bound<PyAny>> {
        match self_.data.get(&key) {
            Some(value) => match value {
                TomlValue::String(s) => Some(PyString::new(self_.py(), s).into_any()),
                TomlValue::Integer(i) => Some(i.into_pyobject(self_.py()).unwrap().into_any()),
                TomlValue::Float(f) => Some(f.into_pyobject(self_.py()).unwrap().into_any()),
                TomlValue::Boolean(b) => {
                    let py_value = PyBool::new(self_.py(), *b);
                    Some(py_value.as_any().clone())
                }
                TomlValue::Array(a) => {
                    let new_self = Self::new(TomlValue::Array(a.clone()));
                    let py_value = new_self.into_pyobject(self_.py()).unwrap().into_any();
                    Some(py_value)
                }
                TomlValue::Table(t) => {
                    let new_self = Self::new(TomlValue::Table(t.clone()));
                    let py_value = new_self.into_pyobject(self_.py()).unwrap().into_any();
                    Some(py_value)
                }
                _ => None,
            },
            None => None,
        }
    }
    pub fn have_key(&self, key: String) -> bool { self.data.get(&key).is_some() }
}

impl ConfigDataPy {
    pub fn new(data: TomlValue) -> Self { Self { data } }
}

#[pymodule]
#[pyo3(name = "shenbot_api")]
fn rs_api_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ConfigDataPy>()?;
    m.add_class::<config::ConfigStoragePy>()?;
    Ok(())
}

/// 在 python 初始化之前注册所有需要的类
///
/// WARNING: 这个函数需要在 Python 初始化之前调用，否则会导致报错
///
/// (pyo3 提供的宏会检查一遍, 不过我这里就直接用原始形式了)
pub fn regist_class() {
    event!(Level::INFO, "向 Python 注册 Rust 侧模块/函数");
    unsafe {
        pyo3::ffi::PyImport_AppendInittab(
            rs_api_module::__PYO3_NAME.as_ptr(),
            Some(rs_api_module::__pyo3_init),
        );
    }

    event!(Level::INFO, "注册完成");
}
