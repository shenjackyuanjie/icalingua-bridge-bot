use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Scratch(PathBuf);

impl Scratch {
    pub(super) fn new() -> io::Result<Self> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("shenbot-process-{}-{now:x}-{seq}", std::process::id()));
        fs::create_dir(&root)?;
        Ok(Self(root))
    }

    pub(super) fn path(&self) -> &Path { &self.0 }
}

impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

#[cfg(windows)]
mod windows {
    use std::{
        fs, io,
        os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
        path::{Path, PathBuf},
        ptr::null,
        time::Duration,
    };

    use tokio::{
        io::AsyncReadExt as _,
        time::{sleep, timeout},
    };
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            CreateEventW, OpenProcess, PROCESS_SYNCHRONIZE, SetEvent, WaitForSingleObject,
        },
    };

    use super::Scratch;
    use crate::ext::process::{ProcessRuntime, windows as managed};

    const LIMIT: Duration = Duration::from_secs(8);

    fn fixture_args() -> Vec<String> {
        let (_, module) = module_path!().split_once("::").unwrap();
        vec![
            "--exact".into(),
            format!("{module}::subprocess_fixture"),
            "--nocapture".into(),
        ]
    }

    async fn wait_for_file(path: &Path) -> io::Result<Vec<u8>> {
        timeout(LIMIT, async {
            loop {
                match fs::read(path) {
                    Ok(bytes) => return Ok(bytes),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        sleep(Duration::from_millis(10)).await
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await?
    }

    fn publish(root: &Path, name: &str, contents: impl AsRef<[u8]>) -> io::Result<()> {
        let temporary = root.join(format!("{name}.tmp"));
        fs::write(&temporary, contents)?;
        fs::rename(temporary, root.join(name))
    }

    async fn open_fixture_process(root: &Path, name: &str) -> io::Result<OwnedHandle> {
        let pid: u32 = String::from_utf8(wait_for_file(&root.join(name)).await?)
            .unwrap()
            .parse()
            .unwrap();
        // 查询正在运行的本机测试子进程；只取等待权限。
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedHandle::from_raw_handle(process) })
    }

    #[test]
    // 模拟主进程不等待后代便退出，故意让 Job 负责回收。
    #[allow(clippy::zombie_processes)]
    fn subprocess_fixture() -> io::Result<()> {
        let cwd = std::env::current_dir()?;
        let root = cwd.join("fixture-control");
        if !root.is_dir() {
            return Ok(());
        }
        let case = fs::read_to_string(cwd.join("fixture-role"))?;
        match case.as_str() {
            "leader" => {
                if let Ok(value) = fs::read_to_string(root.join("sentinel")) {
                    let value = value.parse::<usize>().unwrap();
                    // 若宿主的无关事件句柄被继承，此调用会改变宿主可观察的状态。
                    unsafe { SetEvent(value as *mut core::ffi::c_void) };
                }
                let leaf = root.join("leaf");
                let _leaf = std::process::Command::new(std::env::current_exe()?)
                    .args(fixture_args())
                    .current_dir(leaf)
                    .spawn()?;
                publish(&root, "leader-pid", std::process::id().to_string())?;
                println!("fixture-stdout");
                eprintln!("fixture-stderr");
                while !root.join("exit").exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            "leaf" => {
                let root = cwd.parent().unwrap();
                publish(root, "leaf-pid", std::process::id().to_string())?;
                std::thread::sleep(Duration::from_secs(60));
            }
            "supervisor" => {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                rt.block_on(async {
                    let leader = root.parent().unwrap().join("leader");
                    let runtime = ProcessRuntime::new(&root.join("shadow"))?;
                    let prepared = runtime.prepare(
                        "fixture",
                        1,
                        &std::env::current_exe()?,
                        &fixture_args(),
                        Some(&leader),
                    )?;
                    let _plugin = prepared.spawn()?;
                    sleep(Duration::from_secs(60)).await;
                    Ok::<_, io::Error>(())
                })?;
            }
            _ => return Err(io::Error::other("未知测试进程角色")),
        }
        Ok(())
    }

    fn layout(scratch: &Scratch) -> io::Result<(PathBuf, PathBuf)> {
        let leader = scratch.path().join("leader");
        let root = leader.join("fixture-control");
        let leaf = root.join("leaf");
        fs::create_dir_all(leaf.join("fixture-control"))?;
        fs::write(leader.join("fixture-role"), "leader")?;
        fs::write(leaf.join("fixture-role"), "leaf")?;
        Ok((leader, root))
    }

    #[tokio::test]
    async fn normal_leader_exit_kills_descendant_and_keeps_pipe_tail() -> io::Result<()> {
        let scratch = Scratch::new()?;
        let (leader, control) = layout(&scratch)?;
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        if unsafe {
            SetHandleInformation(event.as_raw_handle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        fs::write(control.join("sentinel"), (event.as_raw_handle() as usize).to_string())?;

        let runtime = ProcessRuntime::new(&scratch.path().join("shadow"))?;
        let prepared = runtime.prepare(
            "fixture",
            1,
            &std::env::current_exe()?,
            &fixture_args(),
            Some(&leader),
        )?;
        let (mut child, _stdin, mut stdout, mut stderr) = managed::spawn(&prepared.0)?;
        let descendant = open_fixture_process(&control, "leaf-pid").await?;
        assert_eq!(unsafe { WaitForSingleObject(descendant.as_raw_handle(), 0) }, WAIT_TIMEOUT);
        assert_eq!(
            unsafe { WaitForSingleObject(event.as_raw_handle(), 0) },
            WAIT_TIMEOUT,
            "子进程不可继承宿主的无关事件"
        );
        fs::write(control.join("exit"), b"")?;
        assert!(timeout(LIMIT, child.wait()).await??.success());
        // Job 的 ActiveProcesses 可在进程句柄最终 signaled 前降为零。
        timeout(LIMIT, async {
            while unsafe { WaitForSingleObject(descendant.as_raw_handle(), 0) } == WAIT_TIMEOUT {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        assert_eq!(unsafe { WaitForSingleObject(descendant.as_raw_handle(), 0) }, WAIT_OBJECT_0);
        let mut output = Vec::new();
        timeout(LIMIT, stdout.read_to_end(&mut output)).await??;
        assert!(String::from_utf8_lossy(&output).contains("fixture-stdout"));
        output.clear();
        timeout(LIMIT, stderr.read_to_end(&mut output)).await??;
        assert!(String::from_utf8_lossy(&output).contains("fixture-stderr"));
        Ok(())
    }

    #[tokio::test]
    async fn killing_supervisor_process_kills_entire_plugin_job() -> io::Result<()> {
        let scratch = Scratch::new()?;
        let (_, control) = layout(&scratch)?;
        fs::create_dir(scratch.path().join("fixture-control"))?;
        fs::write(scratch.path().join("fixture-role"), "supervisor")?;
        let mut host = tokio::process::Command::new(std::env::current_exe()?)
            .args(fixture_args())
            .current_dir(scratch.path())
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()?;
        let leader = open_fixture_process(&control, "leader-pid").await?;
        let descendant = open_fixture_process(&control, "leaf-pid").await?;
        timeout(LIMIT, host.kill()).await??;
        timeout(LIMIT, async {
            loop {
                let leader_status = unsafe { WaitForSingleObject(leader.as_raw_handle(), 0) };
                let leaf_status = unsafe { WaitForSingleObject(descendant.as_raw_handle(), 0) };
                if leader_status == WAIT_OBJECT_0 && leaf_status == WAIT_OBJECT_0 {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        Ok(())
    }
}
