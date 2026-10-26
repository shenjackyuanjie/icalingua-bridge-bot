//! 真实解释器回归由父进程加超时保护，禁止读取用户插件或连接真实服务。
use super::{
    binding::{self, RuntimeBinding},
    call::{self, TaskType},
    host::PluginHost,
    plugin::LifecycleState,
    storage,
};
use crate::{
    config::{BotConfig, PyConfig},
    context::BotContext,
};
use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyModule};
use std::{
    ffi::CString,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    runtime: tokio::runtime::Runtime,
    host: Arc<PluginHost>,
}
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root =
            std::env::temp_dir().join(format!("ica-bot-fixture-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("plugins")).unwrap();
        std::fs::create_dir(root.join("config")).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let config = BotConfig {
            enable_py: true,
            ica: Some(crate::config::IcaConfig {
                private_key: String::new(),
                host: "http://127.0.0.1:9".into(),
                self_id: 10001,
                notice_room: vec![],
                notice_start: false,
                admin_list: vec![],
                filter_list: vec![10003],
            }),
            py: Some(PyConfig {
                plugin_path: root.join("plugins").to_string_lossy().into_owned(),
                config_path: root.join("config").to_string_lossy().into_owned(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let host = PluginHost::new(BotContext::new(config, runtime.handle().clone()).unwrap());
        super::class::regist_class();
        Python::initialize();
        binding::bind(&host).unwrap();
        Self {
            root,
            runtime,
            host,
        }
    }
    fn plugin(&self, id: &str, body: &str) {
        let code = format!(
            "from shenbot_api import PluginManifest, Scheduler, python_plugin_path, python_config_path\nfrom datetime import timedelta\nimport time\nPLUGIN_MANIFEST = PluginManifest({id:?}, {id:?}, 'fixture')\ncalls = 0\nloads = 0\nunloads = 0\ndef on_load():\n    global loads\n    loads += 1\n    assert python_plugin_path()\n    assert python_config_path()\ndef on_unload():\n    global unloads\n    unloads += 1\n{body}\n"
        );
        std::fs::write(self.root.join("plugins").join(format!("{id}.py")), code).unwrap();
    }
    fn load(&self) { self.runtime.block_on(storage::load_plugins(&self.host)).unwrap(); }
    fn module(&self, id: &str) -> Arc<Py<PyModule>> {
        self.runtime
            .block_on(async { self.host.storage.lock().await.storage[id].py_module.clone() })
    }
    fn integer(module: &Py<PyModule>, key: &str) -> i64 {
        Python::attach(|py| module.getattr(py, key).unwrap().extract(py).unwrap())
    }
    fn generation(&self, id: &str) -> u64 {
        self.runtime
            .block_on(async { self.host.storage.lock().await.storage[id].generation() })
    }
    fn internal_module_count() -> usize {
        Python::attach(|py| {
            let expression = CString::new(
                "sum(1 for name in __import__('sys').modules if name.startswith('_shenbot_plugin_'))",
            )
            .unwrap();
            py.eval(&expression, None, None).unwrap().extract().unwrap()
        })
    }
    fn dispatch(&self) {
        self.runtime.block_on(call::call_plugins(
            &self.host,
            TaskType::IcaNewMessage,
            "on_ica_message",
            || {
                (FixtureClient {
                    binding: RuntimeBinding::new(&self.host),
                },)
            },
        ));
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), self.host.tasks.wait_all())
                .await
                .unwrap();
        });
    }
    fn close(&self) { self.runtime.block_on(self.host.shutdown()).unwrap(); }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 只删除本测试创建、位于系统临时目录直接子目录中的唯一目录。
        if let (Ok(root), Ok(temp)) =
            (self.root.canonicalize(), std::env::temp_dir().canonicalize())
            && root.parent() == Some(temp.as_path())
            && root.file_name().unwrap().to_string_lossy().starts_with("ica-bot-fixture-")
        {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
#[pyclass]
struct FixtureClient {
    binding: RuntimeBinding,
}
#[pymethods]
impl FixtureClient {
    fn query(&self, py: Python<'_>) -> PyResult<bool> {
        self.binding.with_host(py, |host| {
            let expected = host.context.runtime.id();
            host.context.runtime.block_on(async {
                // 模拟等待 I/O 后另一工作线程重新进入 Python，遗漏 detach 将死锁。
                tokio::time::sleep(Duration::from_millis(1)).await;
                tokio::task::spawn_blocking(|| Python::attach(|_| ())).await.unwrap();
                host.context.update_ica(|state| {
                    state.loaded_messages_count_by_room.insert(1, 9);
                });
                tokio::runtime::Handle::current().id() == expected
                    && host.context.read_ica(|state| state.loaded_messages_count()) == 9
            })
        })
    }
    fn reload(&self, py: Python<'_>, id: String) -> PyResult<()> {
        self.binding
            .with_host(py, |host| host.context.runtime.block_on(storage::reload_plugin(host, &id)))?
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    }
    fn enabled(&self, py: Python<'_>, id: String) -> PyResult<bool> {
        self.binding
            .with_host(py, |host| host.storage.blocking_lock().get_status(&id).unwrap_or(false))
    }
}

const NORMAL_HOOK: &str =
    "def on_ica_message(client):\n    global calls\n    assert client.query()\n    calls += 1\n";
fn scenario(name: &str) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .try_init();
    let fixture = Fixture::new();
    match name {
        "ica_candidate_before_filter" => {
            fixture.plugin(
                "fixture",
                r#"
last = None
def on_ica_message_candidate(event):
    global calls, last
    calls += 1
    last = event
    assert 'client' not in event
    assert event['account'] == 10001
    assert event['sender_id'] == 10003
    assert event['body'] == '/answer ABC123 1  '
"#,
            );
            fixture.load();
            let msg = crate::ica::event_state::decode_add_message(&serde_json::json!({
                "roomId": -10002, "message": {"_id":"source-fake","senderId":10003,"content":"/answer ABC123 1  ","time":1792688400000_i64,
                    "replyMessage":{"_id":"quote-fake","content":"不能把这个引用正文当成新答案"}}
            })).unwrap();
            let connection = super::ica_connection::ConnectionState {
                generation: 1,
                open: true,
                account: Some(10001),
                online: true,
                client: Some(()),
            };
            assert!(fixture.runtime.block_on(crate::ica::events::before_message_filters(
                &fixture.host,
                &connection,
                1,
                &msg
            )));
            fixture.runtime.block_on(fixture.host.tasks.wait_all());
            assert_eq!(Fixture::integer(&fixture.module("fixture"), "calls"), 1);
            assert!(fixture.runtime.block_on(crate::ica::events::before_message_filters(
                &fixture.host,
                &connection,
                0,
                &msg
            )));
            fixture.runtime.block_on(fixture.host.tasks.wait_all());
            assert_eq!(Fixture::integer(&fixture.module("fixture"), "calls"), 1);
            fixture.close();
        }
        "ica_connection_watch" => {
            fixture.plugin(
                "fixture",
                r#"
from shenbot_api import watch_ica_connection
import queue
import threading
events = queue.Queue()
def monitor(watch):
    while (state := watch.wait_changed()) is not None:
        events.put(state.plugin_active)
def on_load():
    global watch, worker, retained
    watch = watch_ica_connection()
    retained = watch_ica_connection()
    initial = watch.wait_changed()
    assert initial.plugin_active is False
    assert initial.ready is False
    worker = threading.Thread(target=monitor, args=(watch,))
    worker.start()
def on_unload():
    watch.close()
    worker.join(1)
    assert not worker.is_alive()
def await_active():
    assert events.get(timeout=2) is True
"#,
            );
            fixture.load();
            let old = fixture.module("fixture");
            Python::attach(|py| old.getattr(py, "await_active").unwrap().call0(py).unwrap());
            // 没有任何聊天/Bridge 事件，新代提交也必须唤醒 on_load 创建的订阅。
            fixture
                .runtime
                .block_on(storage::reload_plugin(&fixture.host, "fixture"))
                .unwrap();
            let new = fixture.module("fixture");
            Python::attach(|py| {
                new.getattr(py, "await_active").unwrap().call0(py).unwrap();
                let watch = old.getattr(py, "watch").unwrap();
                assert!(watch.call_method0(py, "wait_changed").unwrap().is_none(py));
                let retained = old.getattr(py, "retained").unwrap();
                let error = retained
                    .call_method1(
                        py,
                        "send_text",
                        (0_u64, 10001_i64, -10002_i64, "虚构消息", 0.1_f64),
                    )
                    .unwrap_err();
                assert!(error.to_string().contains("ICA_PLUGIN_GENERATION_STALE"));
                retained.call_method0(py, "close").unwrap();
                let state =
                    new.getattr(py, "retained").unwrap().call_method0(py, "wait_changed").unwrap();
                assert!(state.getattr(py, "plugin_active").unwrap().extract::<bool>(py).unwrap());
                assert!(!state.getattr(py, "ready").unwrap().extract::<bool>(py).unwrap());
            });
            fixture.close();
            Python::attach(|py| {
                let retained = new.getattr(py, "retained").unwrap();
                assert!(retained.call_method0(py, "wait_changed").unwrap().is_none(py));
                retained.call_method0(py, "close").unwrap();
                let api = PyModule::import(py, "shenbot_api").unwrap();
                assert!(api.getattr("watch_ica_connection").unwrap().call0().is_err());
            });
        }
        "ica_connection_watch_capacity" => {
            use super::{call::PluginIdentity, class::ica_connection::IcaConnectionWatchPy};
            let identity = PluginIdentity::new("fixture", 1);
            let mut watches = Vec::new();
            for _ in 0..64 {
                watches.push(IcaConnectionWatchPy::new(&fixture.host, identity.clone()).unwrap());
            }
            assert!(IcaConnectionWatchPy::new(&fixture.host, identity.clone()).is_err());
            watches[0].close();
            watches[0].close();
            assert_eq!(fixture.host.ica_watch_slots.available_permits(), 1);
            let replacement = IcaConnectionWatchPy::new(&fixture.host, identity).unwrap();
            drop(replacement);
            drop(watches);
            assert_eq!(fixture.host.ica_watch_slots.available_permits(), 64);
            fixture.close();
        }
        "received_message" => {
            use crate::py::class::ica::NewMessagePy;
            use pyo3::{ffi::c_str, types::PyDict};
            use serde_json::json;

            for (time, expected_time) in [
                (json!(i64::MAX), None),
                (json!("invalid"), None),
                (json!(null), None),
                (json!(0), Some(0_i64)),
                (json!(1_700_000_000_123_i64), Some(1_700_000_000_123_i64)),
            ] {
                let event = json!({"roomId": -10001, "message": {
                    "_id": "incoming", "username": "测试", "content": "引用旧文件", "time": time,
                    "at": "unknown", "replyMessage": {"_id": "old", "files": [
                        {"type": "application/octet-stream", "fid": "history-fid"}]}
                }});
                let decoded = crate::ica::event_state::decode_add_message(&event).unwrap();
                Python::attach(|py| {
                    let locals = PyDict::new(py);
                    locals
                        .set_item(
                            "msg",
                            Py::new(py, NewMessagePy::new(&decoded, &fixture.host.context))
                                .unwrap(),
                        )
                        .unwrap();
                    locals.set_item("expected_time", expected_time).unwrap();
                    py.run(
                        c_str!(
                            r#"
assert msg.content == "引用旧文件"
assert msg.room_id == -10001
assert msg.at is None
assert msg.time == expected_time
assert msg.reply["_id"] == "old"
assert msg.reply["files"][0]["url"] == ""
assert msg.reply["files"][0]["fid"] == "history-fid"
assert msg.raw_data["at"] == "unknown"
assert msg.to_dict()["at"] is None
"#
                        ),
                        None,
                        Some(&locals),
                    )
                    .unwrap();
                });
            }
            fixture.close();
        }
        "reentry" => {
            fixture.plugin("caller", "def on_ica_message(client):\n    global calls\n    assert client.query()\n    assert client.enabled('peer')\n    client.reload('peer')\n    try:\n        client.reload('caller')\n        raise AssertionError('self reload accepted')\n    except RuntimeError:\n        calls += 1\n");
            fixture.plugin("peer", "");
            fixture.load();
            let old_peer = fixture.module("peer");
            let generation = fixture.generation("peer");
            fixture.dispatch();
            assert_eq!(Fixture::integer(&fixture.module("caller"), "calls"), 1);
            assert_eq!(Fixture::integer(&old_peer, "unloads"), 1);
            assert_eq!(fixture.generation("peer"), generation + 1);
            let facade = FixtureClient {
                binding: RuntimeBinding::new(&fixture.host),
            };
            Python::attach(|py| {
                for _ in 0..20 {
                    assert!(facade.query(py).unwrap());
                }
            });
            fixture.close();
            Python::attach(|py| assert!(facade.query(py).is_err()));
        }
        "module_failure" => {
            let before = Fixture::internal_module_count();
            std::fs::write(fixture.root.join("plugins").join("broken.py"), "def broken(\n")
                .unwrap();
            fixture.runtime.block_on(storage::scan_plugins(&fixture.host)).unwrap();
            assert_eq!(Fixture::internal_module_count(), before);
            assert!(
                fixture
                    .runtime
                    .block_on(async { fixture.host.storage.lock().await.storage.is_empty() })
            );
            fixture.plugin("broken", NORMAL_HOOK);
            fixture.runtime.block_on(storage::scan_plugins(&fixture.host)).unwrap();
            assert_eq!(Fixture::internal_module_count(), before + 1);
            fixture.close();
            assert_eq!(Fixture::internal_module_count(), before);
        }
        "scan" => {
            fixture.plugin("fixture", NORMAL_HOOK);
            fixture.load();
            let old = fixture.module("fixture");
            let generation = fixture.generation("fixture");
            fixture.plugin(
                "fixture",
                "def on_ica_message(client):\n    global calls\n    calls += 10\n",
            );
            fixture.dispatch(); // 文件变更不能由消息触发重载。
            assert_eq!(Fixture::integer(&old, "calls"), 1);
            assert_eq!(fixture.generation("fixture"), generation);
            fixture.runtime.block_on(storage::scan_plugins(&fixture.host)).unwrap();
            fixture.dispatch();
            assert_eq!(Fixture::integer(&fixture.module("fixture"), "calls"), 10);
            assert_eq!(fixture.generation("fixture"), generation + 1);
            let source = fixture.root.join("plugins");
            let target = fixture.root.join("moved-plugins");
            assert!(
                source.parent().unwrap().canonicalize().unwrap()
                    == fixture.root.canonicalize().unwrap()
            );
            assert!(
                target.parent().unwrap().canonicalize().unwrap()
                    == fixture.root.canonicalize().unwrap()
            );
            std::fs::rename(&source, &target).unwrap();
            assert!(fixture.runtime.block_on(storage::scan_plugins(&fixture.host)).is_err());
            assert_eq!(fixture.generation("fixture"), generation + 1);
            std::fs::rename(&target, &source).unwrap();
            std::fs::remove_file(source.join("fixture.py")).unwrap();
            fixture.runtime.block_on(storage::scan_plugins(&fixture.host)).unwrap();
            assert!(
                fixture
                    .runtime
                    .block_on(async { fixture.host.storage.lock().await.storage.is_empty() })
            );
            fixture.close();
        }
        "rollback" => {
            fixture.plugin("fixture", NORMAL_HOOK);
            fixture.load();
            let old = fixture.module("fixture");
            let generation = fixture.generation("fixture");
            fixture.plugin(
                "fixture",
                "def on_load():\n    raise RuntimeError('candidate rejected')\n",
            );
            assert!(
                fixture
                    .runtime
                    .block_on(storage::reload_plugin(&fixture.host, "fixture"))
                    .is_err()
            );
            assert_eq!(fixture.generation("fixture"), generation);
            assert_eq!(Fixture::integer(&old, "loads"), 2);
            fixture.dispatch();
            assert_eq!(Fixture::integer(&old, "calls"), 1);
            fixture
                .runtime
                .block_on(storage::set_plugin_status(&fixture.host, "fixture", false))
                .unwrap();
            fixture.dispatch();
            assert_eq!(Fixture::integer(&old, "calls"), 1);
            fixture
                .runtime
                .block_on(storage::set_plugin_status(&fixture.host, "fixture", true))
                .unwrap();
            fixture.dispatch();
            assert_eq!(Fixture::integer(&old, "calls"), 2);
            fixture.close();
        }
        "scheduler" => {
            fixture.plugin("fixture", "ticks = 0\ndef tick():\n    global ticks\n    ticks += 1\ndef on_load():\n    Scheduler(tick, timedelta(milliseconds=150)).start()\n    time.sleep(0.03)\n");
            fixture.load();
            let old = fixture.module("fixture");
            assert_eq!(Fixture::integer(&old, "ticks"), 0);
            fixture
                .runtime
                .block_on(storage::reload_plugin(&fixture.host, "fixture"))
                .unwrap();
            fixture.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), fixture.host.tasks.wait_all())
                    .await
                    .unwrap();
            });
            assert_eq!(Fixture::integer(&old, "ticks"), 0);
            assert_eq!(Fixture::integer(&fixture.module("fixture"), "ticks"), 1);
            fixture.close();
        }
        "unload_scheduler" => {
            fixture.plugin(
                "fixture",
                "ticks = 0\ndef tick():\n    global ticks\n    ticks += 1\ndef on_unload():\n    try:\n        Scheduler(tick, timedelta(milliseconds=50)).start()\n    except RuntimeError:\n        pass\n",
            );
            fixture.load();
            let old = fixture.module("fixture");
            fixture.plugin(
                "fixture",
                "def on_load():\n    raise RuntimeError('candidate rejected')\n",
            );
            assert!(
                fixture
                    .runtime
                    .block_on(storage::reload_plugin(&fixture.host, "fixture"))
                    .is_err()
            );
            fixture.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), fixture.host.tasks.wait_all())
                    .await
                    .unwrap();
            });
            assert_eq!(Fixture::integer(&old, "ticks"), 0);
            fixture.close();
        }
        "cancel_control" => {
            fixture.plugin("fixture", "unloading = False\ndef on_unload():\n    global unloading\n    unloading = True\n    time.sleep(0.1)\n");
            fixture.load();
            let old = fixture.module("fixture");
            let generation = fixture.generation("fixture");
            let host = fixture.host.clone();
            let caller = fixture
                .runtime
                .spawn(async move { storage::reload_plugin(&host, "fixture").await });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !Python::attach(|py| {
                old.getattr(py, "unloading").unwrap().extract::<bool>(py).unwrap()
            }) {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            caller.abort();
            assert!(fixture.runtime.block_on(caller).unwrap_err().is_cancelled());
            fixture.runtime.block_on(fixture.host.operations.wait_idle());
            assert_eq!(fixture.generation("fixture"), generation + 1);
            assert_eq!(
                fixture.runtime.block_on(async {
                    fixture.host.storage.lock().await.storage["fixture"].state()
                }),
                LifecycleState::Active
            );
            // 关闭也要等待另一个已接纳事务，不能清空后被迟到提交重新插入。
            let current = fixture.module("fixture");
            let host = fixture.host.clone();
            let caller = fixture
                .runtime
                .spawn(async move { storage::reload_plugin(&host, "fixture").await });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !Python::attach(|py| {
                current.getattr(py, "unloading").unwrap().extract::<bool>(py).unwrap()
            }) {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            fixture.close();
            fixture.runtime.block_on(caller).unwrap().unwrap();
            assert!(
                fixture
                    .runtime
                    .block_on(async { fixture.host.storage.lock().await.storage.is_empty() })
            );
        }
        "drain" => {
            fixture.plugin("fixture", NORMAL_HOOK);
            fixture.load();
            let generation = fixture.generation("fixture");
            let identity = super::call::PluginIdentity::new("fixture", generation);
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            assert!(fixture.runtime.block_on(fixture.host.enqueue(identity, move || {
                let _ = started_tx.send(());
                let _ = release_rx.recv();
            })));
            fixture.runtime.block_on(started_rx).unwrap();
            let result = fixture.runtime.block_on(storage::reload_plugin(&fixture.host, "fixture"));
            assert!(result.is_err());
            assert_eq!(fixture.generation("fixture"), generation);
            assert_eq!(
                fixture.runtime.block_on(async {
                    fixture.host.storage.lock().await.storage["fixture"].state()
                }),
                LifecycleState::Active
            );
            release_tx.send(()).unwrap();
            fixture.dispatch();
            assert_eq!(Fixture::integer(&fixture.module("fixture"), "calls"), 1);
            fixture.close();
        }
        _ => panic!("未知测试场景"),
    }
}
#[test]
fn python_host_scenarios_are_isolated_and_bounded() {
    if let Ok(name) = std::env::var("ICA_BOT_FIXTURE_SCENARIO") {
        scenario(&name);
        return;
    }
    for name in [
        "ica_candidate_before_filter",
        "ica_connection_watch",
        "ica_connection_watch_capacity",
        "received_message",
        "reentry",
        "module_failure",
        "scan",
        "rollback",
        "scheduler",
        "unload_scheduler",
        "cancel_control",
        "drain",
    ] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "py::tests::python_host_scenarios_are_isolated_and_bounded",
                "--nocapture",
            ])
            .env("ICA_BOT_FIXTURE_SCENARIO", name)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "Python 场景 {name} 超时：{}\\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "Python 场景 {name} 失败：{}\\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
