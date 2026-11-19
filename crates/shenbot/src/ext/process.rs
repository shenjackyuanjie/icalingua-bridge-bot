//! 进程及管道的所有权与插件生命周期状态机分离。
mod shadow;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use std::{
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::Duration,
};

use ica_ext_sdk::{FrameReader, HostToPlugin, PluginToHost, encode_frame};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use self::shadow::RunDirectory;

const STDIN_CAPACITY: usize = 8;
const READ_BUFFER: usize = 64 * 1024;
const STDERR_PREFIX: usize = 8 * 1024;
const STDERR_DRAIN_GRACE: Duration = Duration::from_millis(100);

pub(super) struct ProcessRuntime {
    run: Arc<RunDirectory>,
}

impl ProcessRuntime {
    pub(super) fn new(shadow_dir: &Path) -> io::Result<Self> {
        Ok(Self {
            run: Arc::new(RunDirectory::new(shadow_dir)?),
        })
    }

    pub(super) fn prepare(
        &self,
        id: &str,
        generation: u64,
        binary: &Path,
        args: &[String],
        working_dir: Option<&Path>,
    ) -> io::Result<PreparedPlugin> {
        let working_dir = working_dir.map(std::fs::canonicalize).transpose()?;
        let executable = self.run.copy_binary(id, generation, binary)?;
        Ok(PreparedPlugin(Arc::new(Prepared {
            executable,
            id: id.to_owned(),
            generation,
            args: args.to_vec(),
            working_dir,
            _run: self.run.clone(),
        })))
    }
}

#[derive(Clone)]
pub(super) struct PreparedPlugin(Arc<Prepared>);

struct Prepared {
    executable: PathBuf,
    id: String,
    generation: u64,
    args: Vec<String>,
    working_dir: Option<PathBuf>,
    _run: Arc<RunDirectory>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.executable)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::debug!(plugin_id = %self.id, generation = self.generation, %error, "运行副本留待下次清扫");
        }
    }
}

pub(super) struct PluginProcess {
    pub(super) stdin: PluginStdin,
    pub(super) stdout: PluginStdout,
    pub(super) child: PluginChild,
}

#[cfg(windows)]
type Child = windows::Child;
#[cfg(not(windows))]
type Child = tokio::process::Child;
#[cfg(windows)]
type ChildStdin = windows::Stdin;
#[cfg(not(windows))]
type ChildStdin = tokio::process::ChildStdin;

impl PreparedPlugin {
    pub(super) fn spawn(&self) -> io::Result<PluginProcess> {
        let prepared = &self.0;
        #[cfg(windows)]
        let (child, stdin, stdout, stderr) = windows::spawn(prepared)?;
        #[cfg(not(windows))]
        let (child, stdin, stdout, stderr) = {
            use std::process::Stdio;

            let mut command = tokio::process::Command::new(&prepared.executable);
            command
                .args(&prepared.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            if let Some(dir) = &prepared.working_dir {
                command.current_dir(dir);
            }
            let mut child = command.spawn()?;
            let stdin = child.stdin.take().expect("已配置 stdin 管道");
            let stdout = child.stdout.take().expect("已配置 stdout 管道");
            let stderr = child.stderr.take().expect("已配置 stderr 管道");
            (child, stdin, stdout, stderr)
        };

        let (tx, rx) = mpsc::channel(STDIN_CAPACITY);
        let writer = tokio::spawn(write_stdin(stdin, rx));
        let stderr = tokio::spawn(log_stderr(stderr, prepared.id.clone(), prepared.generation));
        Ok(PluginProcess {
            stdin: PluginStdin(tx),
            stdout: PluginStdout(FrameReader::new(BufReader::with_capacity(
                READ_BUFFER,
                Box::new(stdout),
            ))),
            child: PluginChild {
                child,
                writer: Some(writer),
                stderr: Some(stderr),
                _prepared: self.clone(),
            },
        })
    }
}

pub(super) struct PluginChild {
    child: Child,
    writer: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
    _prepared: PreparedPlugin,
}

impl PluginChild {
    /// 不消费 stdout；调用者仍可读取进程退出前留下的完整帧。
    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await;
        self.finish_pipes().await;
        status
    }

    pub(super) async fn kill(&mut self) -> io::Result<()> {
        let result = self.child.kill().await;
        self.finish_pipes().await;
        result
    }

    async fn finish_pipes(&mut self) {
        if let Some(writer) = &mut self.writer {
            writer.abort();
            let _ = writer.await;
            self.writer = None;
        }
        if let Some(stderr) = &mut self.stderr {
            // 先给日志尾部机会到达 EOF；若写端意外还活着，不阻塞退场。
            if tokio::time::timeout(STDERR_DRAIN_GRACE, &mut *stderr).await.is_err() {
                stderr.abort();
            }
            self.stderr = None;
        }
    }
}

impl Drop for PluginChild {
    fn drop(&mut self) {
        if let Some(writer) = &self.writer {
            writer.abort();
        }
        if let Some(stderr) = &self.stderr {
            stderr.abort();
        }
    }
}

struct OutgoingFrame {
    bytes: Vec<u8>,
    written: Option<oneshot::Sender<io::Result<()>>>,
}

#[derive(Clone)]
pub(super) struct PluginStdin(mpsc::Sender<OutgoingFrame>);

impl PluginStdin {
    /// 写入 FIFO；仅在完整帧写入子进程的 stdin 后返回成功。
    /// 调用者取消等待不会撤回已经入队的帧。
    pub(super) async fn send(&self, message: &HostToPlugin) -> io::Result<()> {
        let slot = self.0.reserve().await.map_err(|_| stdin_closed())?;
        let bytes = encode_frame(message).map_err(invalid_frame)?;
        let (written, completion) = oneshot::channel();
        slot.send(OutgoingFrame {
            bytes,
            written: Some(written),
        });
        completion.await.map_err(|_| stdin_closed())?
    }

    /// 尝试入队；队列满时立即返回 WouldBlock，成功不表示已写入。
    pub(super) fn try_send(&self, message: &HostToPlugin) -> io::Result<()> {
        let slot = self.0.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "插件 stdin 队列已满")
            }
            mpsc::error::TrySendError::Closed(_) => stdin_closed(),
        })?;
        slot.send(OutgoingFrame {
            bytes: encode_frame(message).map_err(invalid_frame)?,
            written: None,
        });
        Ok(())
    }
}

fn stdin_closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "插件 stdin 已关闭")
}

async fn write_stdin(mut stdin: ChildStdin, mut rx: mpsc::Receiver<OutgoingFrame>) {
    while let Some(frame) = rx.recv().await {
        #[cfg(windows)]
        let result = stdin.write_frame(&frame.bytes).await;
        #[cfg(not(windows))]
        let result = {
            use tokio::io::AsyncWriteExt as _;

            stdin.write_all(&frame.bytes).await
        };
        let failed = result.is_err();
        if let Some(written) = frame.written {
            let _ = written.send(result);
        }
        if failed {
            break;
        }
    }
}

fn invalid_frame(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

pub(super) struct PluginStdout(FrameReader<BufReader<Box<dyn AsyncRead + Unpin + Send>>>);

impl PluginStdout {
    /// 取消安全：在 select 中中断等待后仍保留不完整的帧。
    pub(super) async fn recv(&mut self) -> io::Result<Option<PluginToHost>> {
        self.0.read().await.map_err(invalid_frame)
    }
}

async fn log_stderr(stderr: impl AsyncRead + Unpin, id: String, generation: u64) {
    let mut reader = BufReader::with_capacity(READ_BUFFER, stderr);
    let mut prefix = Vec::new();
    let mut truncated = false;
    let log = |prefix: &[u8], truncated| {
        let prefix = if truncated {
            prefix
        } else {
            prefix.strip_suffix(b"\r").unwrap_or(prefix)
        };
        let message = String::from_utf8_lossy(prefix);
        tracing::info!(plugin_id = %id, generation, truncated, stderr = %message.escape_debug(), "插件日志");
    };
    loop {
        let buf = match reader.fill_buf().await {
            Ok([]) => break,
            Ok(buf) => buf,
            Err(error) => {
                tracing::debug!(plugin_id = %id, generation, %error, "读取插件 stderr 失败");
                break;
            }
        };
        let newline = buf.iter().position(|byte| *byte == b'\n');
        let part = &buf[..newline.unwrap_or(buf.len())];
        let keep = part.len().min(STDERR_PREFIX - prefix.len());
        prefix.extend_from_slice(&part[..keep]);
        truncated |= keep < part.len();
        let consumed = part.len() + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            log(&prefix, truncated);
            prefix.clear();
            truncated = false;
        }
    }
    if !prefix.is_empty() || truncated {
        log(&prefix, truncated);
    }
}
