//! Python 解释器及插件搜索路径初始化。

use std::ffi::CStr;

use anyhow::{Context, anyhow};
use tracing::{Level, event};

/// 初始化 `py_vm`。
pub fn init_py_vm() -> anyhow::Result<()> {
    let cli_args = std::env::args().collect::<Vec<String>>();
    let virtual_env = std::env::var("VIRTUAL_ENV").ok();
    if let Some(env_path) = selected_env(&cli_args, virtual_env.as_deref())? {
        event!(Level::INFO, "使用显式 Python 环境初始化");
        init_py_with_env_path(&env_path)
    } else {
        event!(Level::INFO, "未指定虚拟环境，使用默认 Python 配置");
        init_py_with_default_config()
    }
}

fn selected_env(args: &[String], virtual_env: Option<&str>) -> anyhow::Result<Option<String>> {
    let mut selected = None;
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-env" {
            anyhow::ensure!(selected.is_none(), "-env 只能指定一次");
            let path = args.next().context("-env 缺少虚拟环境路径")?;
            anyhow::ensure!(!path.trim().is_empty() && !path.starts_with('-'), "-env 缺少有效路径");
            selected = Some(path.clone());
        }
    }
    let selected = selected.or_else(|| virtual_env.map(str::to_owned));
    anyhow::ensure!(
        !selected.as_ref().is_some_and(|s| s.trim().is_empty()),
        "VIRTUAL_ENV 不能为空"
    );
    Ok(selected)
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
    let root = std::path::Path::new(path).canonicalize().context("Python 虚拟环境目录不可用")?;
    anyhow::ensure!(root.join("pyvenv.cfg").is_file(), "显式 Python 环境缺少 pyvenv.cfg");
    let interpreter = root.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    anyhow::ensure!(interpreter.is_file(), "显式 Python 环境缺少解释器");
    let path = interpreter.to_str().context("Python 虚拟环境路径不能转换为 Unicode")?;
    unsafe {
        #[cfg(target_os = "windows")]
        use std::ffi::OsStr;
        #[cfg(target_os = "windows")]
        use std::os::windows::ffi::OsStrExt;

        let mut guard = PyConfigGuard::new();
        let config_ptr = &mut guard.config as *mut pyo3::ffi::PyConfig;

        #[cfg(unix)]
        let wide_path = path.chars().map(|c| c as i32).chain(Some(0)).collect::<Vec<i32>>();
        #[cfg(target_os = "windows")]
        let wide_path = OsStr::new(path).encode_wide().chain(Some(0)).collect::<Vec<u16>>();

        check_status(
            pyo3::ffi::PyConfig_SetString(
                config_ptr,
                &mut guard.config.program_name,
                wide_path.as_ptr(),
            ),
            "设置 program_name",
        )?;
        check_status(
            pyo3::ffi::PyConfig_SetString(
                config_ptr,
                &mut guard.config.executable,
                wide_path.as_ptr(),
            ),
            "设置 executable",
        )?;

        check_status(pyo3::ffi::Py_InitializeFromConfig(&guard.config), "Py_InitializeFromConfig")?;
        pyo3::ffi::PyEval_SaveThread();
    }
    event!(Level::INFO, "根据配置初始化 python 完成");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_environment_cannot_silently_fall_back_to_base_python() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("ica-bot-empty-env-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        assert!(
            init_py_with_env_path(root.join("noticer-missing-venv-fictitious").to_str().unwrap())
                .is_err()
        );
        assert!(init_py_with_env_path(root.to_str().unwrap()).is_err());
        assert_eq!(
            root.canonicalize().unwrap().parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir(root).unwrap(); // 只删除本测试新建且仍为空的目录，不递归。
    }

    #[test]
    fn cli_environment_uses_following_value_over_environment_and_rejects_ambiguity() {
        let strings = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        let args = strings(&["bot.exe", "-c", "unused.toml", "-env", "venv 测试"]);
        assert_eq!(selected_env(&args, Some("wrong-env")).unwrap().as_deref(), Some("venv 测试"));
        assert_eq!(
            selected_env(&strings(&["bot.exe"]), Some("chosen-env")).unwrap().as_deref(),
            Some("chosen-env")
        );
        assert!(selected_env(&strings(&["bot.exe"]), None).unwrap().is_none());
        for args in [
            vec!["bot.exe", "-env"],
            vec!["bot.exe", "-env", "-c"],
            vec!["bot.exe", "-env", ""],
            vec!["bot.exe", "-env", "first", "-env", "second"],
        ] {
            assert!(selected_env(&strings(&args), None).is_err());
        }
        assert!(selected_env(&strings(&["bot.exe"]), Some("")).is_err());
    }

    #[test]
    #[ignore = "由 noticer 的制品验证脚本提供全新已安装 wheel 的 venv，单独进程执行"]
    fn embedded_venv_loads_installed_package() {
        use pyo3::{prelude::*, types::PyModule};
        use std::path::Path;
        let root = std::env::var("ICA_BOT_PACKAGE_VENV").expect("需要专用测试虚拟环境");
        let prefix = Path::new(&root).canonicalize().unwrap();
        assert!(prefix.join("pyvenv.cfg").is_file());
        let parent = prefix.parent().unwrap();
        assert!(
            parent
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("noticer-distribution-test-")
        );
        assert_eq!(parent.parent().unwrap(), std::env::temp_dir().canonicalize().unwrap());
        init_py_with_env_path(&root).unwrap();
        Python::attach(|py| {
            let sys = PyModule::import(py, "sys").unwrap();
            let actual: String = sys.getattr("prefix").unwrap().extract().unwrap();
            assert_eq!(Path::new(&actual).canonicalize().unwrap(), prefix);
            let base: String = sys.getattr("base_prefix").unwrap().extract().unwrap();
            assert_ne!(Path::new(&base).canonicalize().unwrap(), prefix);
            let executable: String = sys.getattr("executable").unwrap().extract().unwrap();
            assert!(Path::new(&executable).starts_with(&prefix));
            let package = PyModule::import(py, "noticer_adapter").unwrap();
            let file: String = package.getattr("__file__").unwrap().extract().unwrap();
            assert!(Path::new(&file).canonicalize().unwrap().starts_with(&prefix));
            let websockets = PyModule::import(py, "websockets").unwrap();
            let file: String = websockets.getattr("__file__").unwrap().extract().unwrap();
            assert!(Path::new(&file).canonicalize().unwrap().starts_with(&prefix));
            let site = PyModule::import(py, "site").unwrap();
            assert!(!site.getattr("ENABLE_USER_SITE").unwrap().extract::<bool>().unwrap());
        });
    }
}
