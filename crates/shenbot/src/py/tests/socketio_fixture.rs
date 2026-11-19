//! 由 Noticer 制品脚本单独启动，唯一网络目的地是测试新建的 loopback Bridge。
use super::*;
use std::io::{BufRead, Write};

fn signal(value: &str) {
    println!("NOTICER_FIXTURE:{value}");
    std::io::stdout().flush().unwrap();
}

#[test]
#[ignore = "由 Noticer 制品脚本提供已安装 venv、临时插件和虚构 Socket.IO Bridge"]
fn installed_plugin_socketio_host() {
    let root = PathBuf::from(std::env::var_os("ICA_BOT_SOCKETIO_FIXTURE").expect("需要测试目录"))
        .canonicalize()
        .unwrap();
    assert_eq!(root.parent().unwrap(), std::env::temp_dir().canonicalize().unwrap());
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("noticer-distribution-test-")
    );
    let prefix = root.join("venv 测试").canonicalize().unwrap();
    let port: u16 = std::fs::read_to_string(root.join("bridge.port")).unwrap().parse().unwrap();
    assert_ne!(port, 0);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let ica = crate::config::IcaConfig {
        // RFC 8032 §7.1 TEST 2 的公开测试 seed，绝不能用于实际账号。
        private_key: "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb".into(),
        host: format!("http://127.0.0.1:{port}"),
        self_id: 10001,
        notice_room: vec![],
        notice_start: false,
        admin_list: vec![],
        filter_list: vec![10003],
    };
    let config = BotConfig {
        enable_py: true,
        ica: Some(ica.clone()),
        py: Some(PyConfig {
            plugin_path: root.join("plugins").to_str().unwrap().into(),
            config_path: root.join("config").to_str().unwrap().into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let host = PluginHost::new(BotContext::new(config, runtime.handle().clone()).unwrap());
    crate::STARTUP_TIME.get_or_init(SystemTime::now);
    super::super::class::regist_class();
    super::super::init::init_py_with_env_path(prefix.to_str().unwrap()).unwrap();
    Python::attach(|py| {
        let package = PyModule::import(py, "noticer_adapter").unwrap();
        let file: String = package.getattr("__file__").unwrap().extract().unwrap();
        assert!(PathBuf::from(file).canonicalize().unwrap().starts_with(&prefix));
    });
    binding::bind(&host).unwrap();
    runtime.block_on(storage::load_plugins(&host)).unwrap();
    runtime.block_on(async {
        let plugins = host.storage.lock().await;
        assert_eq!(plugins.storage.len(), 1);
        assert_eq!(plugins.storage["noticer"].state(), LifecycleState::Active);
    });
    let (stop, receiver) = tokio::sync::oneshot::channel();
    let worker = host.clone();
    let connection =
        runtime.spawn(async move { crate::ica::start_ica(&ica, receiver, worker).await });
    signal("ready");
    let mut stopped = false;
    for line in std::io::stdin().lock().lines() {
        match line.unwrap().as_str() {
            "assert-reconnected" => {
                let state = host.ica_connection.current();
                assert_eq!(state.generation, 2, "传输重连必须产生新宿主连接代际");
                assert!(state.ready());
                signal("generation-2");
            }
            "reload" => {
                let (old_thread, old_generation) = runtime.block_on(async {
                    let plugins = host.storage.lock().await;
                    let plugin = &plugins.storage["noticer"];
                    let thread = Python::attach(|py| {
                        plugin
                            .py_module
                            .getattr(py, "_runtime")
                            .unwrap()
                            .getattr(py, "_thread")
                            .unwrap()
                    });
                    (thread, plugin.generation())
                });
                runtime.block_on(storage::reload_plugin(&host, "noticer")).unwrap();
                Python::attach(|py| {
                    assert!(
                        !old_thread
                            .call_method0(py, "is_alive")
                            .unwrap()
                            .extract::<bool>(py)
                            .unwrap()
                    );
                });
                runtime.block_on(async {
                    let plugins = host.storage.lock().await;
                    assert_ne!(plugins.storage["noticer"].generation(), old_generation);
                    assert_eq!(plugins.storage["noticer"].state(), LifecycleState::Active);
                });
                signal("reloaded");
            }
            "stop" => {
                stopped = true;
                break;
            }
            _ => panic!("未知夹具命令"),
        }
    }
    let _ = stop.send(());
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), connection)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(20), host.shutdown())
            .await
            .unwrap()
            .unwrap();
    });
    assert!(stopped, "夹具输入意外中断");
    assert_eq!(host.ica_watch_slots.available_permits(), 64);
    Python::attach(|py| {
        let threading = PyModule::import(py, "threading").unwrap();
        let threads = threading.call_method0("enumerate").unwrap();
        for thread in threads.try_iter().unwrap() {
            let name: String = thread.unwrap().getattr("name").unwrap().extract().unwrap();
            assert!(!name.starts_with("noticer-"), "插件线程未排空");
        }
    });
    signal("stopped");
}
