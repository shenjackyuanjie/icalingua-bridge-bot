use tracing::{Level, event};

pub fn init_py_vm() {
    let cli_args = std::env::args().collect::<Vec<String>>();

    if cli_args.contains(&"-env".to_string()) {
        let env_path = cli_args.iter().find(|&arg| arg != "-env").expect("未找到 -env 参数的值");
        event!(Level::INFO, "找到 -env 参数: {} 正在初始化", env_path);
        // 判断一下是否有 VIRTUAL_ENV 环境变量
        if let Ok(virtual_env) = std::env::var("VIRTUAL_ENV") {
            event!(Level::WARN, "找到 VIRTUAL_ENV 环境变量: {} 将会被 -env 参数覆盖", virtual_env);
        }
        init_py_with_env_path(env_path);
    } else {
        // 根据 VIRTUAL_ENV 环境变量 进行一些处理

        match std::env::var("VIRTUAL_ENV") {
            Ok(virtual_env) => {
                event!(Level::INFO, "找到 VIRTUAL_ENV 环境变量: {} 正在初始化", virtual_env);
                init_py_with_env_path(&virtual_env);
            }
            Err(_) => {
                event!(Level::INFO, "未找到 VIRTUAL_ENV 环境变量, 正常初始化");
                pyo3::prepare_freethreaded_python();
                event!(Level::INFO, "prepare_freethreaded_python 完成");
            }
        }
    }
}

pub fn init_py_with_env_path(path: &str) {
    unsafe {
        #[cfg(target_os = "windows")]
        use std::ffi::OsStr;
        #[cfg(target_os = "windows")]
        use std::os::windows::ffi::OsStrExt;

        let mut config = std::mem::zeroed::<pyo3::ffi::PyConfig>();
        let config_ptr = &mut config as *mut pyo3::ffi::PyConfig;
        // 初始化配置
        // pyo3::ffi::PyConfig_InitIsolatedConfig(config_ptr);
        pyo3::ffi::PyConfig_InitPythonConfig(config_ptr);

        #[cfg(target_os = "linux")]
        let wide_path = path.as_bytes().iter().map(|i| *i as i32).collect::<Vec<i32>>();
        #[cfg(target_os = "windows")]
        let wide_path = OsStr::new(path).encode_wide().chain(Some(0)).collect::<Vec<u16>>();

        // 设置 prefix 和 exec_prefix
        pyo3::ffi::PyConfig_SetString(config_ptr, &mut config.prefix as *mut _, wide_path.as_ptr());
        pyo3::ffi::PyConfig_SetString(
            config_ptr,
            &mut config.exec_prefix as *mut _,
            wide_path.as_ptr(),
        );

        // 使用 Py_InitializeFromConfig 初始化 python
        let status = pyo3::ffi::Py_InitializeFromConfig(&config as *const _);
        pyo3::ffi::PyEval_SaveThread();
        // 清理配置
        pyo3::ffi::PyConfig_Clear(config_ptr);
        match status._type {
            pyo3::ffi::_PyStatus_TYPE::_PyStatus_TYPE_OK => {
                event!(Level::INFO, "根据配置初始化 python 完成");
            }
            pyo3::ffi::_PyStatus_TYPE::_PyStatus_TYPE_EXIT => {
                event!(Level::ERROR, "不对啊, 怎么刚刚初始化 Python 就 EXIT 了");
            }
            pyo3::ffi::_PyStatus_TYPE::_PyStatus_TYPE_ERROR => {
                event!(Level::ERROR, "初始化 python 时发生错误: ERROR");
                pyo3::ffi::Py_ExitStatusException(status);
            }
        }
    }
}
