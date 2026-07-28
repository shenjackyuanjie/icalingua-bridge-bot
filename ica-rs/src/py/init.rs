//! Python 解释器及插件搜索路径初始化。

use std::ffi::CStr;

use anyhow::{Context, anyhow};
use tracing::{Level, event};

/// 初始化 `py_vm`。
pub fn init_py_vm() -> anyhow::Result<()> {
    let cli_args = std::env::args().collect::<Vec<String>>();

    if cli_args.contains(&"-env".to_string()) {
        // 保留既有参数取值行为，本轮只让初始化错误可传播。
        let env_path =
            cli_args.iter().find(|&arg| arg != "-env").context("未找到 -env 参数的值")?;
        event!(Level::INFO, "找到 -env 参数: {} 正在初始化", env_path);
        if let Ok(virtual_env) = std::env::var("VIRTUAL_ENV") {
            event!(Level::WARN, "找到 VIRTUAL_ENV 环境变量: {} 将会被 -env 参数覆盖", virtual_env);
        }
        init_py_with_env_path(env_path)
    } else {
        match std::env::var("VIRTUAL_ENV") {
            Ok(virtual_env) => {
                event!(Level::INFO, "找到 VIRTUAL_ENV 环境变量: {} 正在初始化", virtual_env);
                init_py_with_env_path(&virtual_env)
            }
            Err(_) => {
                event!(Level::INFO, "未找到 VIRTUAL_ENV 环境变量, 正常初始化");
                init_py_with_default_config()
            }
        }
    }
}

fn init_py_with_default_config() -> anyhow::Result<()> {
    unsafe {
        let guard = PyConfigGuard::new();
        check_status(pyo3::ffi::Py_InitializeFromConfig(&guard.config), "Py_InitializeFromConfig")?;
        pyo3::ffi::PyEval_SaveThread();
    }
    event!(Level::INFO, "Python 默认配置初始化完成");
    Ok(())
}

/// 在所有返回路径上清理 CPython 配置。
struct PyConfigGuard {
    config: pyo3::ffi::PyConfig,
}

impl PyConfigGuard {
    unsafe fn new() -> Self {
        let mut config = unsafe { std::mem::zeroed::<pyo3::ffi::PyConfig>() };
        unsafe { pyo3::ffi::PyConfig_InitPythonConfig(&mut config) };
        Self { config }
    }
}

impl Drop for PyConfigGuard {
    fn drop(&mut self) { unsafe { pyo3::ffi::PyConfig_Clear(&mut self.config) }; }
}

/// 把 `PyStatus` 转换为不会终止宿主进程的 Rust 错误。
unsafe fn check_status(status: pyo3::ffi::PyStatus, stage: &str) -> anyhow::Result<()> {
    use pyo3::ffi::_PyStatus_TYPE;

    match status._type {
        _PyStatus_TYPE::_PyStatus_TYPE_OK => Ok(()),
        _PyStatus_TYPE::_PyStatus_TYPE_ERROR | _PyStatus_TYPE::_PyStatus_TYPE_EXIT => {
            let detail = if status.err_msg.is_null() {
                if status._type == _PyStatus_TYPE::_PyStatus_TYPE_EXIT {
                    format!("CPython requested exit with code {}", status.exitcode)
                } else {
                    "CPython returned an unknown initialization error".to_string()
                }
            } else {
                unsafe { CStr::from_ptr(status.err_msg) }.to_string_lossy().into_owned()
            };
            let function = if status.func.is_null() {
                String::new()
            } else {
                format!(" ({})", unsafe { CStr::from_ptr(status.func) }.to_string_lossy())
            };
            Err(anyhow!("Python 初始化阶段 {stage}{function} 失败: {detail}"))
        }
    }
}

/// 使用指定虚拟环境路径初始化 Python。
pub fn init_py_with_env_path(path: &str) -> anyhow::Result<()> {
    unsafe {
        #[cfg(target_os = "windows")]
        use std::ffi::OsStr;
        #[cfg(target_os = "windows")]
        use std::os::windows::ffi::OsStrExt;

        let mut guard = PyConfigGuard::new();
        let config_ptr = &mut guard.config as *mut pyo3::ffi::PyConfig;

        #[cfg(target_os = "linux")]
        let wide_path =
            path.as_bytes().iter().map(|i| *i as i32).chain(Some(0)).collect::<Vec<i32>>();
        #[cfg(target_os = "windows")]
        let wide_path = OsStr::new(path).encode_wide().chain(Some(0)).collect::<Vec<u16>>();

        check_status(
            pyo3::ffi::PyConfig_SetString(config_ptr, &mut guard.config.prefix, wide_path.as_ptr()),
            "设置 prefix",
        )?;
        check_status(
            pyo3::ffi::PyConfig_SetString(
                config_ptr,
                &mut guard.config.exec_prefix,
                wide_path.as_ptr(),
            ),
            "设置 exec_prefix",
        )?;

        check_status(pyo3::ffi::Py_InitializeFromConfig(&guard.config), "Py_InitializeFromConfig")?;
        pyo3::ffi::PyEval_SaveThread();
    }
    event!(Level::INFO, "根据配置初始化 python 完成");
    Ok(())
}
