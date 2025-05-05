use std::collections::HashMap;

use pyo3::{
    Bound, PyAny, PyResult, pyclass, pymethods,
    types::{
        PyAnyMethods, PyBool, PyDict, PyDictMethods, PyFloat, PyInt, PyList, PyListMethods, PyNone,
        PyString, PyStringMethods, PyTuple,
    },
};
use toml::Value as TomlValue;
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
    #[pyo3(get)]
    pub inited: bool,
}

fn parse_py_string(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let py_str = obj.downcast::<PyString>()?;
    let value = py_str.to_str()?;
    Ok(value.to_string())
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
    pub fn init_toml(&self) -> TomlValue {
        let mut root_map = toml::map::Map::with_capacity(self.keys.len());
        for (key, value) in self.keys.iter() {
            let value = &value.default_value;
            match value {
                ConfigItem::None => {}
                ConfigItem::F64(f) => {
                    root_map.insert(key.clone(), TomlValue::Float(*f));
                }
                ConfigItem::I64(i) => {
                    root_map.insert(key.clone(), TomlValue::Integer(*i));
                }
                ConfigItem::String(s) => {
                    root_map.insert(key.clone(), TomlValue::String(s.clone()));
                }
                ConfigItem::Bool(b) => {
                    root_map.insert(key.clone(), TomlValue::Boolean(*b));
                }
                ConfigItem::List(lst) => {
                    let mut vec = Vec::with_capacity(lst.len());
                    for item in lst {
                        match item {
                            ConfigItem::None => {}
                            ConfigItem::F64(f) => vec.push(TomlValue::Float(*f)),
                            ConfigItem::I64(i) => vec.push(TomlValue::Integer(*i)),
                            ConfigItem::String(s) => vec.push(TomlValue::String(s.clone())),
                            ConfigItem::Bool(b) => vec.push(TomlValue::Boolean(*b)),
                            _ => unreachable!("反正检查过了"),
                        }
                    }
                    root_map.insert(key.clone(), TomlValue::Array(vec));
                }
                ConfigItem::Dict(dict) => {
                    let mut map = toml::map::Map::with_capacity(dict.len());
                    for (key, value) in dict {
                        match value {
                            ConfigItem::None => {}
                            ConfigItem::F64(f) => {
                                map.insert(key.clone(), TomlValue::Float(*f));
                            }
                            ConfigItem::I64(i) => {
                                map.insert(key.clone(), TomlValue::Integer(*i));
                            }
                            ConfigItem::String(s) => {
                                map.insert(key.clone(), TomlValue::String(s.clone()));
                            }
                            ConfigItem::Bool(b) => {
                                map.insert(key.clone(), TomlValue::Boolean(*b));
                            }
                            _ => unreachable!("反正检查过了"),
                        }
                    }
                    root_map.insert(key.clone(), TomlValue::Table(map));
                }
            }
        }
        TomlValue::Table(root_map)
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
                for (key, value) in kwargs.iter() {
                    let key = match parse_py_string(&key) {
                        Ok(k) => k,
                        Err(e) => {
                            event!(Level::WARN, "解析配置项名称失败: {:?}\n跳过这一项", e);
                            continue;
                        }
                    };
                    if value.is_instance_of::<PyString>() {
                        keys.insert(
                            key,
                            ConfigItemPy::new_uninit(ConfigItem::String(
                                value.extract::<String>().unwrap(),
                            )),
                        );
                    } else if value.is_instance_of::<PyBool>() {
                        keys.insert(
                            key,
                            ConfigItemPy::new_uninit(ConfigItem::Bool(
                                value.extract::<bool>().unwrap(),
                            )),
                        );
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
                                match parse_py_int(&value) {
                                    Ok(value) => {
                                        items.push(ConfigItem::I64(value));
                                    }
                                    Err(e) => {
                                        event!(
                                            Level::WARN,
                                            "int 解析时出现错误: {}\nraw: {}",
                                            e,
                                            value
                                        );
                                    }
                                }
                            } else if item.is_instance_of::<PyFloat>() {
                                match parse_py_float(&value) {
                                    Ok(value) => {
                                        items.push(ConfigItem::F64(value));
                                    }
                                    Err(e) => {
                                        event!(
                                            Level::WARN,
                                            "float 解析时出现错误: {}\nraw: {}",
                                            e,
                                            value
                                        );
                                    }
                                }
                            } else if item.is_instance_of::<PyBool>() {
                                items.push(ConfigItem::Bool(item.extract::<bool>().unwrap()));
                            } else if item.is_instance_of::<PyNone>() {
                                items.push(ConfigItem::None);
                            } else if item.is_instance_of::<PyTuple>() {
                                event!(Level::WARN, "配置类型不支持 tuple\nraw: {}", item)
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
                                items.insert(
                                    key,
                                    ConfigItem::String(value.extract::<String>().unwrap()),
                                );
                            } else if value.is_instance_of::<PyBool>() {
                                items.insert(
                                    key,
                                    ConfigItem::Bool(value.extract::<bool>().unwrap()),
                                );
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
                            } else if value.is_instance_of::<PyTuple>() {
                                event!(Level::WARN, "配置不支持 Tuple\nraw: {}", value);
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
                Ok(Self {
                    keys,
                    inited: false,
                })
            }
            None => Ok(Self {
                keys: HashMap::new(),
                inited: false,
            }),
        }
    }

    #[pyo3(signature=(key, value, replace=true))]
    pub fn add_item(&mut self, key: &str, value: &Bound<'_, PyAny>, replace: bool) -> bool {
        // 添加配置项
        if self.keys.contains_key(key) && !replace {
            return false;
        }

        let value = {
            if value.is_instance_of::<PyString>() {
                ConfigItemPy::new_uninit(ConfigItem::String(value.extract::<String>().unwrap()))
            } else if value.is_instance_of::<PyBool>() {
                ConfigItemPy::new_uninit(ConfigItem::Bool(value.extract::<bool>().unwrap()))
            } else if value.is_instance_of::<PyFloat>() {
                match value.extract::<f64>() {
                    Ok(v) => ConfigItemPy::new_uninit(ConfigItem::F64(v)),
                    Err(e) => {
                        event!(Level::WARN, "无法解析浮点数: {}", e);
                        return false;
                    }
                }
            } else if value.is_instance_of::<PyInt>() {
                match value.extract::<i64>() {
                    Ok(v) => ConfigItemPy::new_uninit(ConfigItem::I64(v)),
                    Err(e) => {
                        event!(Level::WARN, "无法解析整数: {}", e);
                        return false;
                    }
                }
            } else if value.is_instance_of::<PyList>() {
                let mut items = Vec::new();
                let list = value.downcast::<PyList>().unwrap();
                for item in list.iter() {
                    if item.is_instance_of::<PyString>() {
                        items.push(ConfigItem::String(item.extract::<String>().unwrap()));
                    } else if item.is_instance_of::<PyBool>() {
                        items.push(ConfigItem::Bool(item.extract::<bool>().unwrap()));
                    } else if item.is_instance_of::<PyFloat>() {
                        match item.extract::<f64>() {
                            Ok(v) => items.push(ConfigItem::F64(v)),
                            Err(e) => {
                                event!(Level::WARN, "无法解析浮点数: {}", e);
                            }
                        }
                    } else if item.is_instance_of::<PyInt>() {
                        match item.extract::<i64>() {
                            Ok(v) => items.push(ConfigItem::I64(v)),
                            Err(e) => {
                                event!(Level::WARN, "无法解析整数: {}", e);
                            }
                        }
                    } else if item.is_instance_of::<PyList>() {
                        event!(Level::WARN, "配置项不支持嵌套 List")
                    } else if item.is_instance_of::<PyDict>() {
                        event!(Level::WARN, "配置项不支持嵌套 Dict")
                    } else if item.is_instance_of::<PyTuple>() {
                        event!(Level::WARN, "配置项不支持 Tuple")
                    } else {
                        event!(Level::WARN, "不支持的类型: {}\nraw: {}", item.get_type(), item);
                    }
                }
                ConfigItemPy::new_uninit(ConfigItem::List(items))
            } else if value.is_instance_of::<PyDict>() {
                let mut items = HashMap::new();
                let dict = value.downcast::<PyDict>().unwrap();
                for (key, value) in dict.iter() {
                    let key = match parse_py_string(&key) {
                        Ok(k) => k,
                        Err(e) => {
                            event!(Level::WARN, "解析配置项名称失败: {:?}\n跳过这一项", e);
                            continue;
                        }
                    };
                    if value.is_instance_of::<PyString>() {
                        items.insert(key, ConfigItem::String(value.extract::<String>().unwrap()));
                    } else if value.is_instance_of::<PyBool>() {
                        items.insert(key, ConfigItem::Bool(value.extract::<bool>().unwrap()));
                    } else if value.is_instance_of::<PyFloat>() {
                        match value.extract::<f64>() {
                            Ok(v) => {
                                items.insert(key, ConfigItem::F64(v));
                            }
                            Err(e) => {
                                event!(Level::WARN, "无法解析浮点数: {}", e);
                            }
                        }
                    } else if value.is_instance_of::<PyInt>() {
                        match value.extract::<i64>() {
                            Ok(v) => {
                                items.insert(key, ConfigItem::I64(v));
                            }
                            Err(e) => {
                                event!(Level::WARN, "无法解析整数: {}", e);
                            }
                        }
                    } else if value.is_instance_of::<PyList>() {
                        event!(Level::WARN, "配置项不支持嵌套 List")
                    } else if value.is_instance_of::<PyDict>() {
                        event!(Level::WARN, "配置项不支持嵌套 Dict")
                    } else if value.is_instance_of::<PyTuple>() {
                        event!(Level::WARN, "配置项不支持 Tuple")
                    } else {
                        event!(Level::WARN, "不支持的类型: {}\nraw: {}", value.get_type(), value);
                    }
                }
                ConfigItemPy::new_uninit(ConfigItem::Dict(items))
            } else {
                event!(Level::WARN, "不支持的类型: {}\nraw: {}", value.get_type(), value);
                return false;
            }
        };
        self.keys.insert(key.to_string(), value);
        true
    }
}

#[cfg(test)]
mod tests {
    use pyo3::{PyTypeInfo, Python, ffi::c_str};

    use super::*;

    fn prepare_python() { pyo3::prepare_freethreaded_python(); }

    #[test]
    fn create_config_item() {
        prepare_python();
        Python::with_gil(|py| {
            let locals = PyDict::new(py);
            let _ = locals.set_item("ConfigStorage", ConfigStoragePy::type_object(py));
            let code = c_str!(
                r#"
print(ConfigStorage)
config = ConfigStorage(aaa = "value")
print(config.inited)
"#
            );
            py.run(code, None, Some(&locals)).unwrap();
        })
    }
}
