//! 端到端测试不连接真实 QQ；使用真实副本进程和 SDK，Bridge 动作由测试替身记录。

mod fixture;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ica_ext_sdk::{ConnectionState, HelloAck, HostToPlugin, NewMessage, SendMessage, async_trait};
use shenbot::config::{ExtConfig, ExtPluginConfig, ReloadMode};
use shenbot::ext::{ExtHostServices, ExtPluginHost, ExtPluginState};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, timeout};

use self::fixture::{PIC_ID, PIC_URL, PLUGIN_ID, ROOM_ID};

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("--ext-descendant") {
        std::thread::sleep(Duration::from_secs(90));
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("创建测试运行时");
    if first.as_deref() == Some("--ext-fixture") {
        let mode = args.next().expect("测试夹具模式");
        let directory = args.next().expect("测试夹具目录").into();
        runtime.block_on(fixture::run_plugin(mode, directory));
        return;
    }
    let args: Vec<_> = first.into_iter().chain(args).collect();
    let names = [
        "prefix_action_and_self_reload",
        "serial_rollback_from_shadow",
        "overlap_routes_new_events_and_drains_old_actions",
        "crash_backoff_and_circuit_breaker",
        #[cfg(windows)]
        "shutdown_kills_process_tree",
    ];
    if args.iter().any(|arg| arg == "--list") {
        for name in names {
            println!("{name}: test");
        }
        return;
    }
    let filter = args.iter().find(|arg| !arg.starts_with('-'));
    runtime.block_on(async {
        for name in names.into_iter().filter(|name| filter.is_none_or(|arg| name.contains(arg))) {
            let started = Instant::now();
            timeout(Duration::from_secs(50), async {
                match name {
                    "prefix_action_and_self_reload" => prefix_action_and_self_reload().await,
                    "serial_rollback_from_shadow" => serial_rollback_from_shadow().await,
                    "overlap_routes_new_events_and_drains_old_actions" => {
                        overlap_routes_new_events_and_drains_old_actions().await
                    }
                    "crash_backoff_and_circuit_breaker" => {
                        crash_backoff_and_circuit_breaker().await
                    }
                    #[cfg(windows)]
                    "shutdown_kills_process_tree" => shutdown_kills_process_tree().await,
                    _ => unreachable!(),
                }
            })
            .await
            .unwrap_or_else(|_| panic!("端到端场景 {name} 超时"));
            println!("{name} 通过（{:.2?}）", started.elapsed());
        }
    });
}

enum Effect {
    Message(SendMessage),
    PicUrl(String),
}

struct Services(mpsc::UnboundedSender<Effect>);

#[async_trait]
impl ExtHostServices for Services {
    async fn hello_ack(&self) -> HelloAck {
        HelloAck {
            client_id: "fixture-host".into(),
            self_id: None,
            admin_list: Vec::new(),
            connection: ConnectionState {
                ready: true,
                online: true,
                account: None,
            },
            rooms: Vec::new(),
        }
    }

    async fn send_message(&self, message: SendMessage) -> Result<(), String> {
        self.0.send(Effect::Message(message)).map_err(|err| err.to_string())
    }

    async fn get_nt_pic_url(&self, file_id: String) -> Result<String, String> {
        self.0.send(Effect::PicUrl(file_id)).map_err(|err| err.to_string())?;
        Ok(PIC_URL.into())
    }
}

struct FixtureHost {
    host: Arc<ExtPluginHost>,
    effects: mpsc::UnboundedReceiver<Effect>,
    directory: TestDirectory,
}

impl FixtureHost {
    async fn new(mode: &str, reload_mode: ReloadMode) -> Self {
        let directory = TestDirectory::new(mode);
        let source = directory.path().join("source.exe");
        fs::copy(std::env::current_exe().expect("获取测试程序路径"), &source)
            .expect("创建配置的插件源文件");
        let config = ExtConfig {
            shadow_dir: directory.path().join("shadow"),
            handshake_timeout_ms: 5_000,
            default_drain_timeout_ms: 5_000,
        };
        let units = vec![ExtPluginConfig {
            id: PLUGIN_ID.into(),
            enabled: mode != "self-reload",
            binary: source,
            args: vec![
                "--ext-fixture".into(),
                mode.into(),
                directory.path().to_string_lossy().into_owned(),
            ],
            working_dir: Some(directory.path().to_owned()),
            reload_mode,
            drain_timeout_ms: (mode == "tree").then_some(200),
            auto_restart: mode == "crash",
            allow_self_reload: mode == "self-reload",
        }];
        let (sender, effects) = mpsc::unbounded_channel();
        let host = Arc::new(
            ExtPluginHost::new(config, units, Arc::new(Services(sender)))
                .expect("创建外部插件宿主"),
        );
        host.start_enabled().await;
        Self {
            host,
            effects,
            directory,
        }
    }

    fn dispatch(&self, content: &str) {
        self.host.dispatch(HostToPlugin::NewMessage(NewMessage {
            room_id: ROOM_ID,
            msg_id: "fixture-message".into(),
            sender_id: 100,
            sender_name: "测试用户".into(),
            role: "member".into(),
            content: content.into(),
            time_ms: 0,
            is_from_self: false,
            is_admin: false,
            is_reply: false,
            reply_to: None,
            files: Vec::new(),
        }));
    }

    async fn observe(&mut self) -> Effect {
        timeout(Duration::from_secs(14), self.effects.recv())
            .await
            .expect("未收到外部插件动作")
            .expect("测试动作接收器意外关闭")
    }

    async fn message(&mut self) -> (u32, String) {
        let Effect::Message(message) = self.observe().await else {
            panic!("预期发送消息动作")
        };
        assert_eq!(message.room_id, ROOM_ID);
        assert_eq!(message.reply_to.as_deref(), Some("fixture-message"));
        let text = message.text.expect("测试插件回消息应包含文字");
        let (pid, tag) = text.split_once(':').expect("动作内容应包含进程号");
        (pid.parse().expect("有效的测试进程号"), tag.into())
    }

    async fn expect_message(&mut self, expected: &str) -> u32 {
        let (pid, tag) = self.message().await;
        assert_eq!(tag, expected);
        pid
    }

    fn assert_generation(&self, expected: u64) {
        let summary = &self.host.summaries()[0];
        assert_eq!(summary.state, ExtPluginState::Active, "{summary:?}");
        assert_eq!(summary.generation, Some(expected));
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(mode: &str) -> Self {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("当前系统时间有效")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("shenbot-ext-{mode}-{}-{epoch}", std::process::id()));
        fs::create_dir(&path).expect("创建独占的测试目录");
        Self(path)
    }

    fn path(&self) -> &Path { &self.0 }
}

impl Drop for TestDirectory {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

async fn prefix_action_and_self_reload() {
    let mut fixture = FixtureHost::new("self-reload", ReloadMode::Serial).await;
    assert!(fixture.host.contains(PLUGIN_ID), "禁用状态仍应保留插件 ID");
    let summary = &fixture.host.summaries()[0];
    assert!(!summary.enabled);
    assert_eq!(summary.state, ExtPluginState::Stopped);
    fixture.host.enable(PLUGIN_ID).await.expect("启用已配置插件");
    fixture.assert_generation(1);
    fixture.dispatch("聊天消息里出现了 /fixture picture");
    fixture.dispatch("/fixture-typo picture");
    fixture.dispatch("/fixture picture");
    let Effect::PicUrl(id) = fixture.observe().await else {
        panic!("只有符合前缀的消息才能发起图片动作")
    };
    assert_eq!(id, PIC_ID);
    let old_pid = fixture.expect_message(PIC_URL).await;
    assert!(
        timeout(Duration::from_millis(150), fixture.effects.recv()).await.is_err(),
        "不匹配前缀的消息未经筛选"
    );

    fixture.dispatch("/fixture reload");
    let (first_pid, first) = fixture.message().await;
    let (second_pid, second) = fixture.message().await;
    assert_eq!((first_pid, second_pid), (old_pid, old_pid));
    assert!(
        ["reload-accepted", "shutdown"]
            .into_iter()
            .all(|tag| first == tag || second == tag)
    );
    wait_for(|| fixture.host.summaries()[0].generation == Some(2)).await;
    fixture.dispatch("/fixture ping");
    let new_pid = fixture.expect_message("pong").await;
    assert_ne!(new_pid, old_pid);
    fixture.host.disable(PLUGIN_ID).await.expect("停用已配置插件");
    assert_eq!(fixture.expect_message("shutdown").await, new_pid);
    assert_eq!(fixture.host.summaries()[0].state, ExtPluginState::Stopped);
    assert!(fixture.host.contains(PLUGIN_ID));
    fixture.host.shutdown().await;
}

async fn serial_rollback_from_shadow() {
    let mut fixture = FixtureHost::new("serial", ReloadMode::Serial).await;
    fixture.assert_generation(1);
    fixture.dispatch("/fixture ping");
    let first_pid = fixture.expect_message("pong").await;

    let error = fixture.host.reload(PLUGIN_ID).await.expect_err("新代的 hello ID 与配置不符");
    assert!(error.to_string().contains("已回滚"), "{error}");
    assert_eq!(
        fs::read(fixture.directory.path().join("source.exe")).expect("读取已失效的源文件"),
        b"unavailable"
    );
    assert_eq!(fixture.expect_message("shutdown").await, first_pid);
    fixture.assert_generation(1);
    fixture.dispatch("/fixture ping");
    assert_ne!(fixture.expect_message("pong").await, first_pid);
    fixture.host.shutdown().await;
}

async fn overlap_routes_new_events_and_drains_old_actions() {
    let mut fixture = FixtureHost::new("overlap", ReloadMode::Overlap).await;
    fixture.dispatch("/fixture reload");
    let old_pid = fixture.expect_message("reload-denied").await;
    assert!(fixture.host.reload(PLUGIN_ID).await.is_err(), "坏代不能交换路由");
    fixture.assert_generation(1);
    fixture.dispatch("/fixture ping");
    assert_eq!(fixture.expect_message("pong").await, old_pid);

    fixture.dispatch("/fixture hold");
    assert_eq!(fixture.expect_message("holding").await, old_pid);
    let host = fixture.host.clone();
    let reloading = tokio::spawn(async move { host.reload(PLUGIN_ID).await });
    assert_eq!(fixture.expect_message("shutdown").await, old_pid);
    fixture.assert_generation(3);
    fixture.dispatch("/fixture ping");
    let new_pid = fixture.expect_message("pong").await;
    assert_ne!(new_pid, old_pid);

    fs::write(fixture.directory.path().join("release"), []).expect("让旧代的排队任务继续");
    assert_eq!(fixture.expect_message("old-work").await, old_pid);
    assert_eq!(fixture.expect_message("old-work-acked").await, old_pid);
    reloading.await.expect("等待重载任务").expect("热切应完成");
    fixture.host.shutdown().await;
    assert_eq!(fixture.expect_message("shutdown").await, new_pid);
}

async fn crash_backoff_and_circuit_breaker() {
    let mut fixture = FixtureHost::new("crash", ReloadMode::Serial).await;
    let mut previous_pid = fixture.expect_message("boot").await;
    for secs in [1, 2, 4, 8] {
        let start = Instant::now();
        let pid = fixture.expect_message("boot").await;
        assert_ne!(pid, previous_pid);
        // 观察方可能略迟于进程收到回执，留少量调度容差；退避的精确窗口另有单元验证。
        assert!(start.elapsed() >= Duration::from_millis(secs * 1_000 - 200));
        previous_pid = pid;
    }
    wait_for(|| fixture.host.summaries()[0].state == ExtPluginState::Failed).await;
    assert_eq!(fixture.host.summaries()[0].generation, None);
    fixture.host.shutdown().await;
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(10), async {
        while !condition() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("外部插件状态未按期切换");
}

#[cfg(windows)]
async fn shutdown_kills_process_tree() {
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use windows_sys::Win32::Foundation::STILL_ACTIVE;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn open(pid: u32) -> OwnedHandle {
        // pid 来自测试夹具。保留句柄直到断言完成，避免把 PID 复用误判成残留。
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        assert!(!handle.is_null(), "{}", std::io::Error::last_os_error());
        // OpenProcess 成功返回当前调用方独占所有权的进程句柄。
        unsafe { OwnedHandle::from_raw_handle(handle) }
    }

    let mut fixture = FixtureHost::new("tree", ReloadMode::Serial).await;
    let (parent_pid, data) = fixture.message().await;
    let child_pid = data
        .strip_prefix("descendant:")
        .expect("夹具必须启动一个后代")
        .parse()
        .expect("有效的后代进程号");
    let handles = [open(parent_pid), open(child_pid)];
    fixture.host.shutdown().await;
    for handle in handles {
        let mut status = 0;
        // 仍持有有效句柄，且 status 指向有效的本地变量。
        assert_ne!(unsafe { GetExitCodeProcess(handle.as_raw_handle(), &mut status) }, 0);
        assert_ne!(status, STILL_ACTIVE as u32, "宿主关闭后不应残留插件进程或其后代");
    }
}
