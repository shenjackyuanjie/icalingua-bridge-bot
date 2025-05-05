use std::collections::HashMap;

use pyo3::{
    Bound, PyAny, PyResult, pyclass, pymethods,
    types::{
        PyAnyMethods, PyBool, PyBoolMethods, PyDict, PyDictMethods, PyFloat, PyInt, PyList,
        PyListMethods, PyNone, PyString, PyStringMethods, PyTuple, PyTypeMethods,
    },
};
use tracing::{Level, event};

/// 配置项类型
#[derive(Debug, Clone)]
pub enum ConfigItem {
    None,
    // 直接 value
    String(String),
    I64(i64),
    F64(f64),
    Bool(bool),
    /// 数组
    ///
    /// 不支持嵌套, 支持混杂
    List(Vec<ConfigItem>),
    /// map
    ///
    /// 不支持嵌套, 支持混杂
    Dict(HashMap<String, ConfigItem>),
}

#[derive(Clone, Debug)]
#[pyclass]
#[pyo3(name = "ConfigItem")]
pub struct ConfigItemPy {
    pub item: Option<ConfigItem>,
    pub default_value: ConfigItem,
}

impl ConfigItemPy {
    pub fn new(item: Option<ConfigItem>, default_value: ConfigItem) -> Self {
        Self {
            item,
            default_value,
        }
    }

    pub fn new_uninit(default_value: ConfigItem) -> Self {
        Self {
            item: None,
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

#[pymethods]
impl ConfigStoragePy {
    #[new]
    #[pyo3(signature = (**kwargs))]
    pub fn new(kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        match kwargs {
            Some(kwargs) => {
                let mut keys = HashMap::new();
                // 解析 kwargs
                for (key, value) in kwargs.iter() {
                    let key = match parse_py_string(&key) {
                        Ok(k) => k,
                        Err(e) => {
                            event!(Level::WARN, "解析配置项名称失败: {:?}\n跳过这一项", e);
                            continue;
                        }
                    };
                    if value.is_instance_of::<PyString>() {
                        match parse_py_string(&value) {
                            Ok(value) => {
                                keys.insert(
                                    key,
                                    ConfigItemPy::new_uninit(ConfigItem::String(value)),
                                );
                            }
                            Err(e) => {
                                event!(
                                    Level::WARN,
                                    "{}(string) 解析时出现错误: {}\nraw: {}",
                                    key,
                                    e,
                                    value
                                );
                            }
                        }
                    } else if value.is_instance_of::<PyBool>() {
                        match parse_py_bool(&value) {
                            Ok(value) => {
                                keys.insert(key, ConfigItemPy::new_uninit(ConfigItem::Bool(value)));
                            }
                            Err(e) => {
                                event!(
                                    Level::WARN,
                                    "{}(bool) 解析时出现错误: {}\nraw: {}",
                                    key,
                                    e,
                                    value
                                );
                            }
                        }
                    } else if value.is_instance_of::<PyInt>() {
                        match parse_py_int(&value) {
                            Ok(value) => {
                                keys.insert(key, ConfigItemPy::new_uninit(ConfigItem::I64(value)));
                            }
                            Err(e) => {
                                event!(
                                    Level::WARN,
                                    "{}(int) 解析时出现错误: {}\nraw: {}",
                                    key,
                                    e,
                                    value
                                );
                            }
                        }
                    } else if value.is_instance_of::<PyFloat>() {
                        match parse_py_float(&value) {
                            Ok(value) => {
                                keys.insert(key, ConfigItemPy::new_uninit(ConfigItem::F64(value)));
                            }
                            Err(e) => {
                                event!(
                                    Level::WARN,
                                    "{}(float) 解析时出现错误: {}\nraw: {}",
                                    key,
                                    e,
                                    value
                                );
                            }
                        }
                    } else if value.is_instance_of::<PyNone>() {
                        // none: 无默认值
                        keys.insert(key, ConfigItemPy::new_uninit(ConfigItem::None));
                    } else if value.is_instance_of::<PyList>() {
                        // list: 那么几个玩意的列表
                        let list = value.downcast::<PyList>().unwrap();
                        let mut items = Vec::new();
                        for item in list.iter() {
                            if item.is_instance_of::<PyString>() {
                                items.push(ConfigItem::String(item.extract::<String>().unwrap()));
                            } else if item.is_instance_of::<PyInt>() {
                                items.push(ConfigItem::I64(item.extract::<i64>().unwrap()));
                            } else if item.is_instance_of::<PyFloat>() {
                                items.push(ConfigItem::F64(item.extract::<f64>().unwrap()));
                            } else if item.is_instance_of::<PyBool>() {
                                items.push(ConfigItem::Bool(item.extract::<bool>().unwrap()));
                            } else if item.is_instance_of::<PyNone>() {
                                items.push(ConfigItem::None);
                            } else if item.is_instance_of::<PyList>() {
                                event!(Level::WARN, "配置类型不支持嵌套 List\nraw: {}", item)
                            } else if item.is_instance_of::<PyDict>() {
                                event!(Level::WARN, "配置类型不支持嵌套 Dict\nraw: {}", item)
                            } else {
                                event!(
                                    Level::WARN,
                                    "不支持的列表元素类型: {}\nraw: {}",
                                    item.get_type(),
                                    item
                                );
                            }
                        }
                        keys.insert(key, ConfigItemPy::new_uninit(ConfigItem::List(items)));
                    } else if value.is_instance_of::<PyDict>() {
                        let dict = value.downcast::<PyDict>().unwrap();
                        let mut items = HashMap::new();
                        for (key, value) in dict {
                            let key = match parse_py_string(&key) {
                                Ok(k) => k,
                                Err(e) => {
                                    event!(Level::WARN, "解析配置项名称失败: {:?}\n跳过这一项", e);
                                    continue;
                                }
                            };
                            if value.is_instance_of::<PyString>() {
                                match parse_py_string(&value) {
                                    Ok(value) => {
                                        items.insert(key, ConfigItem::String(value));
                                    }
                                    Err(e) => {
                                        event!(
                                            Level::WARN,
                                            "{}(string) 解析时出现错误: {}\nraw: {}",
                                            key,
                                            e,
                                            value
                                        );
                                    }
                                }
                            } else if value.is_instance_of::<PyBool>() {
                                match parse_py_bool(&value) {
                                    Ok(value) => {
                                        items.insert(key, ConfigItem::Bool(value));
                                    }
                                    Err(e) => {
                                        event!(
                                            Level::WARN,
                                            "{}(bool) 解析时出现错误: {}\nraw: {}",
                                            key,
                                            e,
                                            value
                                        );
                                    }
                                }
                            } else if value.is_instance_of::<PyInt>() {
                                match parse_py_int(&value) {
                                    Ok(value) => {
                                        items.insert(key, ConfigItem::I64(value));
                                    }
                                    Err(e) => {
                                        event!(
                                            Level::WARN,
                                            "{}(int) 解析时出现错误: {}\nraw: {}",
                                            key,
                                            e,
                                            value
                                        );
                                    }
                                }
                            } else if value.is_instance_of::<PyFloat>() {
                                match parse_py_float(&value) {
                                    Ok(value) => {
                                        items.insert(key, ConfigItem::F64(value));
                                    }
                                    Err(e) => {
                                        event!(
                                            Level::WARN,
                                            "{}(float) 解析时出现错误: {}\nraw: {}",
                                            key,
                                            e,
                                            value
                                        );
                                    }
                                }
                            } else if value.is_instance_of::<PyNone>() {
                                // none: 无默认值
                                items.insert(key, ConfigItem::None);
                            }
                        }
                        keys.insert(key, ConfigItemPy::new_uninit(ConfigItem::Dict(items)));
                    } else if value.is_instance_of::<PyTuple>() {
                        event!(Level::WARN, "配置不支持 Tuple\nraw: {}", value)
                    } else {
                        event!(
                            Level::WARN,
                            "不支持的值({})类型: {}\nraw: {}",
                            key,
                            value.get_type(),
                            value
                        );
                    }
                }
                // 解析完成
                Ok(Self { keys })
            }
            None => Ok(Self {
                keys: HashMap::new(),
            }),
        }
    }
}
