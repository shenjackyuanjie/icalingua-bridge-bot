use std::{
    cell::UnsafeCell,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        fs::OpenOptionsExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, IntoRawHandle as _, OwnedHandle},
    },
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::net::windows::named_pipe::NamedPipeServer;
use windows_sys::Win32::{
    Foundation::{ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND,
        PIPE_ACCESS_OUTBOUND, SECURITY_IDENTIFICATION, WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_REJECT_REMOTE_CLIENTS},
    },
};

static PIPE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Pipe {
    server: OwnedHandle,
    peer: File,
}

impl Pipe {
    pub(super) fn new(parent_reads: bool) -> io::Result<Self> {
        let seq = PIPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let name = format!(r"\\.\pipe\shenbot-{pid:x}-{now:x}-{seq:x}");
        let wide: Vec<_> = name.encode_utf16().chain([0]).collect();
        let access = if parent_reads {
            PIPE_ACCESS_INBOUND
        } else {
            PIPE_ACCESS_OUTBOUND
        };
        // 宿主端使用 overlapped I/O，子端仍是可供 stdio 使用的同步句柄。
        // 两端初始不可继承；只允许本机客户端，且只能存在一个实例。
        let server = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                access | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_REJECT_REMOTE_CLIENTS,
                1,
                64 * 1024,
                64 * 1024,
                0,
                null(),
            )
        };
        if server == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // API 成功时把新句柄的所有权交给调用者。
        let server = unsafe { OwnedHandle::from_raw_handle(server) };
        let peer = OpenOptions::new()
            .read(!parent_reads)
            .write(parent_reads)
            .share_mode(0)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name)?;
        // CreateFile 已连接。即使其他本机进程抢先连接，也不接受其数据。
        let mut client_pid = 0;
        if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle(), &mut client_pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if client_pid != pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "插件管道的连接方不是宿主",
            ));
        }
        Ok(Self { server, peer })
    }

    pub(super) fn into_stdin(self) -> (Stdin, File) { (Stdin(self.server), self.peer) }

    pub(super) fn into_reader(self) -> io::Result<(NamedPipeServer, File)> {
        // server 是已连接、支持 overlapped 的独占服务端句柄；转让所有权。
        let reader = unsafe { NamedPipeServer::from_raw_handle(self.server.into_raw_handle()) }?;
        Ok((reader, self.peer))
    }
}

pub(in crate::ext::process) struct Stdin(OwnedHandle);

impl Stdin {
    pub(in crate::ext::process) async fn write_frame(
        &mut self,
        mut bytes: &[u8],
    ) -> io::Result<()> {
        while !bytes.is_empty() {
            let written = PendingWrite::start(&self.0, bytes)?.complete().await?;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            bytes = &bytes[written..];
        }
        Ok(())
    }
}

// Tokio/Mio 的命名管道写入在提交而非完成时返回，且析构不会取消写入。
// stdin 自行追踪完成状态：成功意味着整帧已写入管道，而非仅提交 I/O。
struct PendingWrite<'a> {
    pipe: &'a OwnedHandle,
    _bytes: &'a [u8],
    overlapped: Box<UnsafeCell<OVERLAPPED>>,
    pending: bool,
}

// 状态与缓冲区借用随 future 移动；OVERLAPPED 地址固定且只通过 Win32 访问。
// 每条 stdin 独占操作，完成或撤消前不会销毁缓冲区和 OVERLAPPED。
unsafe impl Send for PendingWrite<'_> {}

impl<'a> PendingWrite<'a> {
    fn start(pipe: &'a OwnedHandle, bytes: &'a [u8]) -> io::Result<Self> {
        let len = u32::try_from(bytes.len()).map_err(io::Error::other)?;
        let mut operation = Self {
            pipe,
            _bytes: bytes,
            overlapped: Box::new(UnsafeCell::new(OVERLAPPED::default())),
            pending: false,
        };
        // bytes 和 OVERLAPPED 保持有效直至完成；一条管道仅有这一个操作，
        // hEvent 可为 NULL，此时等待 GetOverlappedResult 使用管道本身的信号。
        if unsafe {
            WriteFile(
                pipe.as_raw_handle(),
                bytes.as_ptr(),
                len,
                null_mut(),
                operation.overlapped.get(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
        }
        operation.pending = true;
        Ok(operation)
    }

    async fn complete(mut self) -> io::Result<usize> {
        loop {
            match self.result(false) {
                Err(error) if error.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) => {
                    // 只有遇到子进程背压时才等待，且不占用 Tokio 阻塞线程。
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                result => {
                    self.pending = false;
                    return result.map(|written| written as usize);
                }
            }
        }
    }

    fn result(&self, wait: bool) -> io::Result<u32> {
        let mut written = 0;
        // 此结构只保存成功提交给该句柄的操作；系统写入局部的 written。
        if unsafe {
            GetOverlappedResult(
                self.pipe.as_raw_handle(),
                self.overlapped.get(),
                &mut written,
                wait.into(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(written)
    }
}

impl Drop for PendingWrite<'_> {
    fn drop(&mut self) {
        if self.pending {
            // 请求撤消并收割结果后才销毁内存；操作可能已在撤消前正常完成。
            unsafe { CancelIoEx(self.pipe.as_raw_handle(), self.overlapped.get()) };
            let _ = self.result(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancels_pending_stdin_with_peer_still_open() -> io::Result<()> {
        use std::io::Read as _;

        let (mut stdin, mut peer) = Pipe::new(false)?.into_stdin();
        stdin.write_frame(b"ready").await?;
        let mut received = [0; 5];
        peer.read_exact(&mut received)?;
        assert_eq!(&received, b"ready");

        let frame = vec![b'x'; 2 * 1024 * 1024];
        let operation = PendingWrite::start(&stdin.0, &frame)?;
        assert_eq!(
            operation.result(false).unwrap_err().raw_os_error(),
            Some(ERROR_IO_INCOMPLETE as i32)
        );
        // 超时销毁 future 时，读端仍打开，不能依赖子进程退出解除阻塞。
        assert!(
            tokio::time::timeout(Duration::from_millis(30), operation.complete())
                .await
                .is_err()
        );
        // 同一写端还能继续提交操作；说明上一次撤消已收割。
        let second = PendingWrite::start(&stdin.0, &frame)?;
        assert_eq!(
            second.result(false).unwrap_err().raw_os_error(),
            Some(ERROR_IO_INCOMPLETE as i32)
        );
        drop(second);
        Ok(())
    }
}
