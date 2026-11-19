mod pipe;

use std::{
    ffi::OsStr,
    io,
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
        process::ExitStatusExt as _,
    },
    process::ExitStatus,
    ptr::{null, null_mut},
    sync::Arc,
    time::Duration,
};

use tokio::{net::windows::named_pipe::NamedPipeServer, task::JoinHandle};

use self::pipe::Pipe;
pub(super) use self::pipe::Stdin;
use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0},
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

use super::Prepared;

pub(super) fn spawn(
    prepared: &Prepared,
) -> io::Result<(Child, Stdin, NamedPipeServer, NamedPipeServer)> {
    let application = wide_z(prepared.executable.as_os_str())?;
    let mut command_line = command_line(prepared)?;
    let directory = prepared.working_dir.as_ref().map(|p| wide_z(p.as_os_str())).transpose()?;
    let current_dir = directory.as_ref().map_or(null(), |directory| directory.as_ptr());

    let job = Job::new()?;
    let (stdin, child_stdin) = Pipe::new(false)?.into_stdin();
    let (stdout, child_stdout) = Pipe::new(true)?.into_reader()?;
    let (stderr, child_stderr) = Pipe::new(true)?.into_reader()?;
    let inherited = [
        child_stdin.as_raw_handle(),
        child_stdout.as_raw_handle(),
        child_stderr.as_raw_handle(),
    ];
    let jobs = [job.handle.as_raw_handle()];
    let mut attributes = Attributes::new()?;
    // 两个数组在 CreateProcessW 返回之前保持有效。只继承三条 stdio 的子端。
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherited[0];
    startup.StartupInfo.hStdOutput = inherited[1];
    startup.StartupInfo.hStdError = inherited[2];
    startup.lpAttributeList = attributes.as_ptr();
    let mut process = PROCESS_INFORMATION::default();

    // Win32 要求白名单中的句柄可继承；到最后一步才开启，且父端和 Job 始终不可继承。
    for handle in inherited {
        // 句柄由上面的 OwnedHandle 持有。
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    // 原子加入 Job，连「创建后、首条插件指令前宿主被强杀」也不会留下未入组进程。
    // 句柄和参数均在此调用期间有效；允许 Windows 修改 command_line 缓冲区。
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
            null(),
            current_dir,
            &startup.StartupInfo,
            &mut process,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // 成功时 Win32 把两个独立句柄的所有权交给调用者。
    let (process_handle, thread_handle) = unsafe {
        (
            OwnedHandle::from_raw_handle(process.hProcess),
            OwnedHandle::from_raw_handle(process.hThread),
        )
    };
    drop((thread_handle, child_stdin, child_stdout, child_stderr));
    Ok((
        Child {
            process: Arc::new(process_handle),
            job: Some(job),
            waiter: None,
            status: None,
        },
        stdin,
        stdout,
        stderr,
    ))
}

pub(super) struct Child {
    process: Arc<OwnedHandle>,
    job: Option<Job>,
    waiter: Option<JoinHandle<io::Result<ExitStatus>>>,
    status: Option<ExitStatus>,
}

impl Child {
    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        if self.status.is_none() {
            let waiter = self.waiter.get_or_insert_with(|| {
                let process = self.process.clone();
                tokio::task::spawn_blocking(move || wait_for_process(&process))
            });
            let status = waiter.await.map_err(io::Error::other).and_then(|status| status);
            self.waiter = None;
            match status {
                Ok(status) => self.status = Some(status),
                Err(error) => {
                    drop(self.job.take());
                    return Err(error);
                }
            }
        }
        if let Some(job) = &mut self.job {
            let result = job.finish().await;
            // finish() 失败时仍关掉最后一个 Job 句柄，触发 KILL_ON_JOB_CLOSE。
            drop(self.job.take());
            result?;
        }
        Ok(self.status.expect("已取得插件退出状态"))
    }

    pub(super) async fn kill(&mut self) -> io::Result<()> {
        if let Some(job) = &mut self.job
            && let Err(error) = job.terminate()
        {
            drop(self.job.take());
            let _ = self.wait().await;
            return Err(error);
        }
        self.wait().await.map(|_| ())
    }
}

fn wait_for_process(process: &OwnedHandle) -> io::Result<ExitStatus> {
    // 所有等待共用同一个任务，取消 wait 不会丢失进程状态；句柄在任务退出前有效。
    match unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } {
        WAIT_OBJECT_0 => {}
        WAIT_FAILED => return Err(io::Error::last_os_error()),
        value => return Err(io::Error::other(format!("等待插件进程结果异常: {value}"))),
    }
    let mut status = 0;
    // 句柄已 signaled，读取最终退出码。
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut status) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ExitStatus::from_raw(status))
}

struct Job {
    handle: OwnedHandle,
    terminated: bool,
}

impl Job {
    fn new() -> io::Result<Self> {
        // 未命名、不可继承的 Job，只有当前宿主拥有句柄。
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // 该句柄由 CreateJobObjectW 新建并交给调用者。
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // 按 JobObjectExtendedLimitInformation 的 ABI 传入完整结构。
        if unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            terminated: false,
        })
    }

    fn terminate(&mut self) -> io::Result<()> {
        if !self.terminated {
            // 该句柄仍由宿主持有，可终止整个 Job（含嵌套 Job 的进程）。
            if unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            self.terminated = true;
        }
        Ok(())
    }

    async fn finish(&mut self) -> io::Result<()> {
        self.terminate()?;
        loop {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // 退出主进程不足以排空后代；等待 Job 中最后一个进程真正退出。
            if unsafe {
                QueryInformationJobObject(
                    self.handle.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&raw mut info).cast(),
                    size_of_val(&info) as u32,
                    null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if info.ActiveProcesses == 0 {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

struct Attributes(Vec<usize>);

impl Attributes {
    fn new() -> io::Result<Self> {
        let mut size = 0;
        // 以 NULL 查询两个属性所需容量。用 usize 缓冲区保证 Win32 所需对齐。
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
        // 缓冲区至少具有所需大小，且后续不会被重新分配。
        if unsafe { InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 2, 0, &mut size) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buffer))
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST { self.0.as_mut_ptr().cast() }

    fn set(&mut self, attribute: u32, handles: &[HANDLE]) -> io::Result<()> {
        // 两个调用点保证 handles 活到 CreateProcessW 完成以及属性表释放。
        if unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                attribute as usize,
                handles.as_ptr().cast(),
                size_of_val(handles),
                null_mut(),
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // Attributes 只在 Initialize 成功后构造；释放 API 内部状态而非 Vec 的分配。
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn wide_z(value: &OsStr) -> io::Result<Vec<u16>> {
    let raw: Vec<_> = value.encode_wide().collect();
    if raw.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "路径或参数包含 NUL"));
    }
    Ok(raw.into_iter().chain([0]).collect())
}

fn command_line(prepared: &Prepared) -> io::Result<Vec<u16>> {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let mut line = Vec::new();
    // argv[0] 用 Windows 的可执行文件路径语法；文件名本身不允许包含双引号。
    let application = wide_z(prepared.executable.as_os_str())?;
    line.push(QUOTE);
    line.extend_from_slice(&application[..application.len() - 1]);
    line.push(QUOTE);
    for arg in &prepared.args {
        line.extend([b' ' as u16, QUOTE]);
        let mut backslashes = 0;
        for code in arg.encode_utf16() {
            match code {
                0 => return Err(io::Error::new(io::ErrorKind::InvalidInput, "参数包含 NUL")),
                BACKSLASH => backslashes += 1,
                _ => {
                    // 紧邻引号的反斜杠需要加倍，且转义参数内的引号。
                    let count = if code == QUOTE {
                        2 * backslashes + 1
                    } else {
                        backslashes
                    };
                    line.extend(std::iter::repeat_n(BACKSLASH, count));
                    line.push(code);
                    backslashes = 0;
                }
            }
        }
        line.extend(std::iter::repeat_n(BACKSLASH, 2 * backslashes));
        line.push(QUOTE);
    }
    line.push(0);
    Ok(line)
}
