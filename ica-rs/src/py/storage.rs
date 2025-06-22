use std::collections::HashMap;

use crate::py::plugin::PyPlugin;


#[derive(Debug)]
pub struct PyPluginStorage {
    pub stogage: HashMap<String, PyPlugin>
}

impl PyPluginStorage {
    pub fn new() -> Self { Self {
        stogage: HashMap::new()
    } }
}
