use std::collections::HashMap;

use pyo3::{
    pyclass, pymethods,
    types::{
        PyAnyMethods, PyBool, PyBoolMethods, PyDict, PyDictMethods, PyFloat, PyInt, PyList,
        PyListMethods, PyString, PyStringMethods, PyTypeMethods,
    },
    Bound, PyAny, PyResult,
};
use tracing::{event, Level};

#[derive(Debug, Clone)]
pub enum ConfigItem {
    None,
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<ConfigItemPy>),
    Table(HashMap<String, ConfigItemPy>),
}

#[derive(Clone, Debug)]
#[pyclass]
#[pyo3(name = "ConfigItem")]
pub struct ConfigItemPy {
    pub item: ConfigItem,
    pub default_value: ConfigItem,
}

impl ConfigItemPy {
    pub fn new(item: ConfigItem, default_value: ConfigItem) -> Self {
        Self {
            item,
            default_value,
        }
    }

    pub fn new_uninit(default_value: ConfigItem) -> Self {
        Self {
            item: ConfigItem::None,
            default_value,
        }
    }
}

#[derive(Clone)]
#[pyclass]
#[pyo3(name = "ConfigStorage")]
pub struct ConfigStoragePy {
    pub keys: HashMap<String, ConfigItemPy>,
}

/// Storage 里允许的最大层级深度
///
/// 我也不知道为啥就突然有这玩意了(
pub const MAX_CFG_DEPTH: usize = 10;

fn parse_py_string(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let py_str = obj.downcast::<PyString>()?;
    let value = py_str.to_str()?;
    Ok(value.to_string())
}

fn parse_py_bool(obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py_bool = obj.downcast::<PyBool>()?;
    Ok(py_bool.is_true())
}

fn parse_py_int(obj: &Bound<'_, PyAny>) -> PyResult<i64> {
    let py_int = obj.downcast::<PyInt>()?;
    py_int.extract::<i64>()
}

fn parse_py_float(obj: &Bound<'_, PyAny>) -> PyResult<f64> {
    let py_float = obj.downcast::<PyFloat>()?;
    py_float.extract::<f64>()
}

impl ConfigStoragePy {
    /// 递归 list 解析配置
    ///
    /// 用个 Result 来标记递归过深
    fn parse_py_list(
        args: &Bound<'_, PyList>,
        list: &mut Vec<ConfigItemPy>,
        current_deepth: usize,
    ) -> Result<(), usize> {
        if current_deepth > MAX_CFG_DEPTH {
            return Err(current_deepth);
        } else {
            for value in args.iter() {
                // 匹配 item
                let value_type = value.get_type();
                if value_type.is_instance_of::<PyDict>() {
                    let py_dict = value.downcast::<PyDict>().unwrap();
                    let mut new_map = HashMap::new();
                    match Self::parse_py_dict(py_dict, &mut new_map, current_deepth + 1) {
                        Ok(_) => {
                            list.push(ConfigItemPy::new_uninit(ConfigItem::Table(new_map)));
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "value(dict) 解析时出现错误: {}\nraw: {}",
                                e,
                                value
                            );
                        }
                    }
                } else if value_type.is_instance_of::<PyList>() {
                    let py_list = value.downcast::<PyList>().unwrap();
                    let mut new_list = Vec::new();
                    match Self::parse_py_list(py_list, &mut new_list, current_deepth + 1) {
                        Ok(_) => {
                            list.push(ConfigItemPy::new_uninit(ConfigItem::Array(new_list)));
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "value(list) 解析时出现错误: {}\nraw: {}",
                                e,
                                value
                            );
                        }
                    }
                } else if value_type.is_instance_of::<PyString>() {
                    match parse_py_string(&value) {
                        Ok(value) => {
                            list.push(ConfigItemPy::new_uninit(ConfigItem::String(value)));
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "value(string) 解析时出现错误: {}\nraw: {}",
                                e,
                                value
                            );
                        }
                    }
                } else if value_type.is_instance_of::<PyBool>() {
                    match parse_py_bool(&value) {
                        Ok(value) => {
                            list.push(ConfigItemPy::new_uninit(ConfigItem::Bool(value)));
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "value(bool) 解析时出现错误: {}\nraw: {}",
                                e,
                                value
                            );
                        }
                    }
                } else if value_type.is_instance_of::<PyInt>() {
                    match parse_py_int(&value) {
                        Ok(value) => {
                            list.push(ConfigItemPy::new_uninit(ConfigItem::Int(value)));
                        }
                        Err(e) => {
                            event!(Level::WARN, "value(int) 解析时出现错误: {}\nraw: {}", e, value);
                        }
                    }
                } else if value_type.is_instance_of::<PyFloat>() {
                    match parse_py_float(&value) {
                        Ok(value) => {
                            list.push(ConfigItemPy::new_uninit(ConfigItem::Float(value)));
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "value(float) 解析时出现错误: {}\nraw: {}",
                                e,
                                value
                            );
                        }
                    }
                } else {
                    // 先丢个 warning 出去
                    match value_type.name() {
                        Ok(type_name) => {
                            event!(
                                Level::WARN,
                                "value 为不支持的 {} 类型\nraw: {}",
                                type_name,
                                value
                            )
                        }
                        Err(e) => {
                            event!(
                                Level::WARN,
                                "value 为不支持的类型 (获取类型名失败: {})\nraw: {}",
                                e,
                                value
                            )
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 递归 dict 解析配置
    ///
    /// 用个 Result 来标记递归过深
    fn parse_py_dict(
        kwargs: &Bound<'_, PyDict>,
        map: &mut HashMap<String, ConfigItemPy>,
        current_deepth: usize,
    ) -> Result<(), usize> {
        if current_deepth > MAX_CFG_DEPTH {
            Err(current_deepth)
        } else {
            for (key, value) in kwargs.iter() {
                if let Ok(name) = key.downcast::<PyString>() {
                    let name = name.to_string();
                    // 匹配 item
                    let value_type = value.get_type();
                    if value_type.is_instance_of::<PyDict>() {
                        let py_dict = value.downcast::<PyDict>().unwrap();
                        let mut new_map = HashMap::new();
                        match Self::parse_py_dict(py_dict, &mut new_map, current_deepth + 1) {
                            Ok(_) => {
                                map.insert(
                                    name.clone(),
                                    ConfigItemPy::new_uninit(ConfigItem::Table(new_map)),
                                );
                            }
                            Err(e) => {
                                event!(Level::WARN, "value(dict) {} 解析时出现错误: {}", name, e);
                            }
                        }
                    } else if value_type.is_instance_of::<PyList>() {
                        let py_list = value.downcast::<PyList>().unwrap();
                        let mut new_list = Vec::new();
                        match Self::parse_py_list(py_list, &mut new_list, current_deepth + 1) {
                            Ok(_) => {
                                map.insert(
                                    name.clone(),
                                    ConfigItemPy::new_uninit(ConfigItem::Array(new_list)),
                                );
                            }
                            Err(e) => {
                                event!(Level::WARN, "value(list) {} 解析时出现错误: {}", name, e);
                            }
                        }
                    } else if value_type.is_instance_of::<PyString>() {
                        match parse_py_string(&value) {
                            Ok(value) => {
                                map.insert(
                                    name.clone(),
                                    ConfigItemPy::new_uninit(ConfigItem::String(value)),
                                );
                            }
                            Err(e) => {
                                event!(Level::WARN, "value(string) {} 解析时出现错误: {}", name, e);
                            }
                        }
                    } else if value_type.is_instance_of::<PyBool>() {
                        match parse_py_bool(&value) {
                            Ok(value) => {
                                map.insert(
                                    name.clone(),
                                    ConfigItemPy::new_uninit(ConfigItem::Bool(value)),
                                );
                            }
                            Err(e) => {
                                event!(Level::WARN, "value(bool) {} 解析时出现错误: {}", name, e);
                            }
                        }
                    } else if value_type.is_instance_of::<PyInt>() {
                        match parse_py_int(&value) {
                            Ok(value) => {
                                map.insert(
                                    name.clone(),
                                    ConfigItemPy::new_uninit(ConfigItem::Int(value)),
                                );
                            }
                            Err(e) => {
                                event!(Level::WARN, "value(int) {} 解析时出现错误: {}", name, e);
                            }
                        }
                    } else if value_type.is_instance_of::<PyFloat>() {
                        match parse_py_float(&value) {
                            Ok(value) => {
                                map.insert(
                                    name.clone(),
                                    ConfigItemPy::new_uninit(ConfigItem::Float(value)),
                                );
                            }
                            Err(e) => {
                                event!(Level::WARN, "value(float) {} 解析时出现错误: {}", name, e);
                            }
                        }
                    } else {
                        // 先丢个 warning 出去
                        match value_type.name() {
                            Ok(type_name) => {
                                event!(Level::WARN, "value {} 为不支持的 {} 类型", name, type_name)
                            }
                            Err(e) => event!(
                                Level::WARN,
                                "value {} 为不支持的类型 (获取类型名失败: {})",
                                name,
                                e
                            ),
                        }
                        continue;
                    }
                }
            }
            Ok(())
        }
    }
}

#[pymethods]
impl ConfigStoragePy {
    #[new]
    #[pyo3(signature = (**kwargs))]
    pub fn new(kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        match kwargs {
            Some(kwargs) => {
                let mut keys = HashMap::new();
                // 解析 kwargs
                Self::parse_py_dict(kwargs, &mut keys, 0).map_err(|e| {
                    event!(Level::ERROR, "配置解析过深: {}", e);
                    pyo3::exceptions::PyValueError::new_err(format!("配置解析过深: {}", e))
                })?;
                // 解析完成
                Ok(Self { keys })
            }
            None => Ok(Self {
                keys: HashMap::new(),
            }),
        }
    }

    #[getter]
    /// 获取最大允许的层级深度
    pub fn get_max_allowed_depth(&self) -> usize { MAX_CFG_DEPTH }
}
