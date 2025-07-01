use std::collections::HashMap;

use pyo3::{
    Bound, IntoPyObjectExt, PyAny, PyObject, PyResult, Python, pyclass, pymethods,
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
    /// none
    None,
    /// string
    String(String),
    /// i64
    I64(i64),
    /// f64
    F64(f64),
    /// bool
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

impl ConfigItem {
    pub fn str(str: impl ToString) -> Self { ConfigItem::String(str.to_string()) }

    pub fn bool(b: bool) -> Self { ConfigItem::Bool(b) }

    fn inner_from_toml(value: &TomlValue, layer: u32) -> Option<Self> {
        match value {
            TomlValue::String(str) => Some(Self::str(str)),
            TomlValue::Boolean(b) => Some(Self::bool(*b)),
            TomlValue::Float(f) => Some(Self::F64(*f)),
            TomlValue::Integer(i) => Some(Self::I64(*i)),
            TomlValue::Datetime(d) => {
                event!(Level::WARN, "暂时还不支持用 datetime! 直接给你换成 string 了");
                Some(Self::str(d))
            }
            TomlValue::Array(lst) => {
                if layer != 0 {
                    event!(Level::WARN, "哥们不允许嵌套!");
                    None
                } else {
                    Some(Self::List(
                        lst.iter()
                            .enumerate()
                            .filter_map(|(idx, item)| {
                                Self::inner_from_toml(item, layer + 1).inspect(|_| ()).or_else(
                                    || {
                                        event!(Level::WARN, "解析 list 元素失败 index = {}", idx);
                                        None
                                    },
                                )
                            })
                            .collect(),
                    ))
                }
            }
            TomlValue::Table(dict) => {
                if layer != 0 {
                    event!(Level::WARN, "哥们不允许嵌套!");
                    None
                } else {
                    Some(Self::Dict(
                        dict.into_iter()
                            .filter_map(|(key, value)| {
                                Self::inner_from_toml(value, layer + 1)
                                    .map(|val| (key.clone(), val))
                            })
                            .collect(),
                    ))
                }
            }
        }
    }

    pub fn from_toml(value: &TomlValue) -> Option<Self> { Self::inner_from_toml(value, 0) }

    pub fn as_py_obj(&self, py: Python) -> PyObject {
        match &self {
            ConfigItem::None => PyNone::get(py).to_owned().into_any(),
            ConfigItem::String(str) => PyString::new(py, str).into_any(),
            ConfigItem::I64(i) => PyInt::new(py, *i).into_any(),
            ConfigItem::F64(f) => PyFloat::new(py, *f).into_any(),
            ConfigItem::Bool(b) => b.into_bound_py_any(py).unwrap(),
            ConfigItem::List(lst) => {
                PyList::new(py, lst.iter().map(|i| i.as_py_obj(py))).unwrap().into_any()
            }
            ConfigItem::Dict(map) => {
                let py_map = PyDict::new(py);
                for (key, value) in map.iter() {
                    let _ = py_map.set_item(key, value.as_py_obj(py));
                }
                py_map.into_any()
            }
        }
        .unbind()
    }
}

#[derive(Clone, Debug)]
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

    pub fn sync_default(&mut self) { self.item = Some(self.default_value.clone()) }

    pub fn read_toml(&mut self, value: &TomlValue) {
        match &self.default_value {
            ConfigItem::None => self.item = ConfigItem::from_toml(value),
            ConfigItem::Dict(..) => {
                if let Some(table) = value.as_table() {
                    let keys = table
                        .into_iter()
                        .filter_map(|(key, item)| {
                            ConfigItem::from_toml(item).map(|val| (key.clone(), val))
                        })
                        .collect();
                    self.item = Some(ConfigItem::Dict(keys));
                } else {
                    event!(Level::WARN, "toml 类型 {} 和默认类型不匹配 (dict)", value.type_str())
                }
            }
            ConfigItem::List(..) => {
                if let Some(lst) = value.as_array() {
                    let data = lst.iter().filter_map(ConfigItem::from_toml).collect();
                    self.item = Some(ConfigItem::List(data));
                } else {
                    event!(Level::WARN, "toml 类型 {} 和默认类型不匹配 (list)", value.type_str())
                }
            }
            _ => {
                if value.is_array() || value.is_table() {
                    event!(Level::WARN, "默认值不是 dict/list, toml 给了一个 {}", value.type_str())
                } else {
                    self.item = ConfigItem::from_toml(value)
                }
            }
        }
    }

    pub fn have_item(&self, name: &str) -> bool {
        if let Some(item) = &self.item {
            matches!(item, ConfigItem::Dict(map) if map.contains_key(name))
        } else {
            false
        }
    }

    pub fn as_py_obj(&self, py: Python<'_>) -> Option<PyObject> {
        self.item.as_ref().map(|item| item.as_py_obj(py))
    }
}

#[derive(Clone, Debug)]
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
    pub fn as_toml(&self, default: bool) -> toml::Table {
        let mut root_map = toml::map::Map::with_capacity(self.keys.len());
        for (key, value) in self.keys.iter() {
            let value = if default {
                &value.default_value
            } else {
                match &value.item {
                    None => continue,
                    Some(value) => value,
                }
            };
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
        root_map
    }

    /// 读取 toml 文件
    ///
    /// 会覆盖现有内容
    pub fn read_toml(&mut self, map: &toml::Table) {
        // 检查 default, 看看有没有对应 key
        for (default_key, inner_value) in self.keys.iter_mut() {
            if let Some(value) = map.get(default_key) {
                inner_value.read_toml(value);
            } else {
                event!(Level::INFO, "toml 缺失 {} 键, 使用默认值", default_key);
                inner_value.sync_default();
            }
        }
        self.inited = true;
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
                            ConfigItemPy::new_uninit(ConfigItem::str(
                                value.extract::<String>().unwrap(),
                            )),
                        );
                    } else if value.is_instance_of::<PyBool>() {
                        keys.insert(
                            key,
                            ConfigItemPy::new_uninit(ConfigItem::bool(
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
                                    ConfigItem::str(value.extract::<String>().unwrap()),
                                );
                            } else if value.is_instance_of::<PyBool>() {
                                items.insert(
                                    key,
                                    ConfigItem::bool(value.extract::<bool>().unwrap()),
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
                ConfigItemPy::new_uninit(ConfigItem::str(value.extract::<String>().unwrap()))
            } else if value.is_instance_of::<PyBool>() {
                ConfigItemPy::new_uninit(ConfigItem::bool(value.extract::<bool>().unwrap()))
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

    pub fn get_default_toml(&self) -> String {
        let value = self.as_toml(true);
        toml::to_string_pretty(&value).unwrap()
    }

    pub fn get_current_toml(&self) -> String {
        let value = self.as_toml(false);
        toml::to_string_pretty(&value).unwrap()
    }

    pub fn read_toml_str(&mut self, value: &str) -> anyhow::Result<()> {
        let parsed_toml: toml::Table = toml::from_str(value)?;
        self.read_toml(&parsed_toml);
        Ok(())
    }

    #[pyo3(signature = (layer1, layer2=None))]
    pub fn have_value(&self, layer1: &str, layer2: Option<&str>) -> bool {
        if !self.inited {
            return false;
        }
        if let Some(item) = self.keys.get(layer1) {
            if let Some(layer) = layer2 {
                return item.have_item(layer);
            }
            return true;
        }
        false
    }

    #[pyo3(signature = (layer1, layer2=None))]
    pub fn get_value(
        &self,
        py: Python<'_>,
        layer1: &str,
        layer2: Option<&str>,
    ) -> Option<PyObject> {
        if !self.inited {
            return None;
        }
        if let Some(item) = self.keys.get(layer1) {
            if let Some(layer) = layer2 {
                if let Some(item) = &item.item {
                    match item {
                        ConfigItem::Dict(map) => map.get(layer).map(|v| v.as_py_obj(py)),
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                item.as_py_obj(py)
            }
        } else {
            None
        }
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
config = ConfigStorage(aaa = "value", aaaa = "value", cc=2)
print(config.inited)
print(config.get_default_toml())
"#
            );
            py.run(code, None, Some(&locals)).unwrap();
        })
    }

    /// 其实就是我自己忘记下一步要怎么写了, 遂先写个 test 再说
    #[test]
    fn parse_toml_test() {
        prepare_python();
        let toml_value = toml::toml! {
            abc = 12333
            some_map.val = "string"
            password = "123456"
        };
        Python::with_gil(|py| {
            let locals = PyDict::new(py);
            let _ = locals.set_item("ConfigStorage", ConfigStoragePy::type_object(py));
            // 用 python 初始化
            let code = c_str!(
                r#"test = ConfigStorage(abc=100,
    bcd=200,
    some_map={"val_2": 123},
    username=None,
    password=None,)"#
            );
            py.run(code, None, Some(&locals)).unwrap();
            // 然后在怪费劲的拿出来
            // 很费劲啊？
            let mut obj = locals
                .get_item("test")
                .ok()
                .flatten()
                .unwrap()
                .extract::<ConfigStoragePy>()
                .unwrap();
            obj.read_toml(&toml_value);

            let parsed_value = obj.as_toml(false);
            let correct_toml_value = toml::toml! {
                abc = 12333
                bcd = 200
                // some map 被覆盖
                some_map.val = "string"
                password = "123456"
            };
            assert_eq!(parsed_value, correct_toml_value);

            assert!(obj.inited);
            let _ = locals.set_item("test", obj);
            let code = c_str!("print(test.inited)");
            py.run(code, None, Some(&locals)).unwrap();
        });
    }
}
