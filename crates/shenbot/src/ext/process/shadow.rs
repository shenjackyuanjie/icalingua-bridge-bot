use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const ROOT_LOCK: &str = ".shenbot-runtime.lock";
const LEASE: &str = ".lease";
const LEASE_MARKER: &[u8] = b"shenbot-shadow-v1\n";
const SESSION_PREFIX: &str = "shenbot-run-";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct RunDirectory {
    root: PathBuf,
    path: PathBuf,
    lease: Option<File>,
    #[cfg(windows)]
    _root_guard: File,
    #[cfg(windows)]
    dir_guard: Option<File>,
}

impl RunDirectory {
    pub(super) fn new(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        #[cfg(windows)]
        let root_guard = open_runtime_dir(root)?;
        #[cfg(not(windows))]
        if !plain_dir(&fs::symlink_metadata(root)?) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "运行副本目录不能是链接"));
        }
        let root = fs::canonicalize(root)?;
        // 扫描和发布新目录串行，避免其他宿主误删尚未获得租约的目录。
        let _guard = lock_root(&root)?;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name().to_str().is_some_and(is_session_name)
                && let Err(error) = cleanup_orphan(&entry.path())
            {
                tracing::debug!(%error, "未清理运行副本目录");
            }
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let path = loop {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = root.join(format!(
                "{SESSION_PREFIX}{:08x}-{now:032x}-{sequence:016x}",
                std::process::id()
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        #[cfg(windows)]
        let dir_guard = open_runtime_dir(&path)?;
        let lease = match open_lock(&path.join(LEASE), false, true).and_then(|mut file| {
            lock(&file, true)?;
            file.write_all(LEASE_MARKER)?;
            Ok(file)
        }) {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_file(path.join(LEASE));
                #[cfg(windows)]
                drop(dir_guard);
                let _ = fs::remove_dir(&path);
                return Err(error);
            }
        };
        Ok(Self {
            root,
            path,
            lease: Some(lease),
            #[cfg(windows)]
            _root_guard: root_guard,
            #[cfg(windows)]
            dir_guard: Some(dir_guard),
        })
    }

    pub(super) fn copy_binary(
        &self,
        id: &str,
        generation: u64,
        binary: &Path,
    ) -> io::Result<PathBuf> {
        // Windows 文件名不区分大小写；保留小写 ID 的可读名称，其他 ID 映射到独立命名空间。
        let id =
            if id.len() <= 128 && plain_id(id) && !id.bytes().any(|byte| byte.is_ascii_uppercase())
            {
                id.to_owned()
            } else {
                format!("~{}", blake3::hash(id.as_bytes()).to_hex())
            };
        let dest = self.path.join(format!("{id}-gen{generation}.exe"));
        let mut source = File::open(binary)?;
        let metadata = source.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "插件二进制不是普通文件"));
        }
        let mut copy = OpenOptions::new().write(true).create_new(true).open(&dest)?;
        let result = io::copy(&mut source, &mut copy);
        #[cfg(unix)]
        let result = result.and_then(|_| copy.set_permissions(metadata.permissions()));
        drop(copy);
        if let Err(error) = result {
            let _ = fs::remove_file(&dest);
            return Err(error);
        }
        Ok(dest)
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let Ok(_guard) = lock_root(&self.root) else {
            return;
        };
        drop(self.lease.take());
        #[cfg(windows)]
        drop(self.dir_guard.take());
        let _ = cleanup_orphan(&self.path);
    }
}

fn lock_root(root: &Path) -> io::Result<File> {
    let file = open_lock(&root.join(ROOT_LOCK), true, false)?;
    lock(&file, true)?;
    Ok(file)
}

fn open_lock(path: &Path, create: bool, create_new: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create).create_new(create_new);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let file = options.open(path)?;
    if !plain_file(&file.metadata()?) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "租约不是普通文件"));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_runtime_dir(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    // 持有不共享 DELETE 的非重解析目录句柄，防止清扫期间目录被重命名或替换。
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)?;
    if !plain_dir(&dir.metadata()?) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "运行副本目录必须是普通目录"));
    }
    Ok(dir)
}

fn cleanup_orphan(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    let dir_guard = open_runtime_dir(path)?;
    #[cfg(not(windows))]
    if !plain_dir(&fs::symlink_metadata(path)?) {
        return Ok(());
    }
    let lease_path = path.join(LEASE);
    let mut lease = match open_lock(&lease_path, false, false) {
        Ok(lease) => lease,
        // 宿主在创建空目录之后、发布租约之前崩溃。只尝试删除空目录。
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(windows)]
            drop(dir_guard);
            return fs::remove_dir(path);
        }
        Err(error) => return Err(error),
    };
    if !lock(&lease, false)? {
        return Ok(());
    }
    let mut marker = [0; LEASE_MARKER.len()];
    if lease.metadata()?.len() != marker.len() as u64 {
        return Ok(());
    }
    lease.read_exact(&mut marker)?;
    if marker != LEASE_MARKER {
        return Ok(());
    }

    let mut copies = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == LEASE {
            continue;
        }
        // 清扫不递归，且遇到任何无关文件或 reparse point 时保留整目录。
        if !name.to_str().is_some_and(is_shadow_name)
            || !plain_file(&fs::symlink_metadata(entry.path())?)
        {
            return Ok(());
        }
        copies.push(entry.path());
    }
    for copy in copies {
        fs::remove_file(copy)?;
    }
    drop(lease);
    fs::remove_file(lease_path)?;
    #[cfg(windows)]
    drop(dir_guard);
    fs::remove_dir(path)
}

fn is_session_name(name: &str) -> bool {
    fn hex(value: &str, width: usize) -> bool {
        value.len() == width && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }
    let Some(pieces) = name.strip_prefix(SESSION_PREFIX) else {
        return false;
    };
    let mut parts = pieces.split('-');
    matches!((parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(pid), Some(now), Some(sequence), None) if hex(pid, 8) && hex(now, 32) && hex(sequence, 16))
}

fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn is_shadow_name(name: &str) -> bool {
    let Some((id, generation)) =
        name.strip_suffix(".exe").and_then(|name| name.rsplit_once("-gen"))
    else {
        return false;
    };
    let safe_id =
        (id.len() <= 128 && plain_id(id) && !id.bytes().any(|byte| byte.is_ascii_uppercase()))
            || id.strip_prefix('~').is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            });
    safe_id && generation.parse::<u64>().is_ok_and(|number| number.to_string() == generation)
}

fn plain_dir(metadata: &Metadata) -> bool { metadata.is_dir() && !is_reparse(metadata) }

fn plain_file(metadata: &Metadata) -> bool { metadata.is_file() && !is_reparse(metadata) }

#[cfg(windows)]
fn is_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(metadata: &Metadata) -> bool { metadata.is_symlink() }

#[cfg(windows)]
fn lock(file: &File, wait: bool) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::{
        Foundation::ERROR_LOCK_VIOLATION,
        Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
        System::IO::OVERLAPPED,
    };
    let mut flags = LOCKFILE_EXCLUSIVE_LOCK;
    if !wait {
        flags |= LOCKFILE_FAIL_IMMEDIATELY;
    }
    let mut overlapped = OVERLAPPED::default();
    // 文件在整个调用期间有效，且未以 FILE_FLAG_OVERLAPPED 打开；锁随句柄释放。
    if unsafe { LockFileEx(file.as_raw_handle(), flags, 0, 1, 0, &mut overlapped) } != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if !wait && error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        return Ok(false);
    }
    Err(error)
}

#[cfg(unix)]
fn lock(file: &File, wait: bool) -> io::Result<bool> {
    use std::os::fd::AsRawFd as _;
    let flags = if wait {
        libc::LOCK_EX
    } else {
        libc::LOCK_EX | libc::LOCK_NB
    };
    loop {
        // 文件描述符在调用期间有效；锁随描述符释放。
        if unsafe { libc::flock(file.as_raw_fd(), flags) } == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::WouldBlock if !wait => return Ok(false),
            io::ErrorKind::Interrupted => continue,
            _ => return Err(error),
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn lock(_file: &File, _wait: bool) -> io::Result<bool> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "平台不支持运行副本租约"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::process::{ProcessRuntime, tests::Scratch};

    #[test]
    fn cleanup_preserves_live_and_unowned_files() -> io::Result<()> {
        let root = Scratch::new()?;
        let binary = root.path().join("input.exe");
        fs::write(&binary, b"sample executable")?;
        let runtime = ProcessRuntime::new(root.path())?;
        let prepared = runtime.prepare("test", 1, &binary, &[], None)?;
        let live = &prepared.0.executable;
        let session = live.parent().unwrap().to_owned();

        let orphan = root
            .path()
            .join("shenbot-run-00000000-00000000000000000000000000000000-0000000000000000");
        fs::create_dir(&orphan)?;
        fs::write(orphan.join(LEASE), LEASE_MARKER)?;
        fs::write(orphan.join("test-gen1.exe"), b"stale copy")?;

        let foreign = root
            .path()
            .join("shenbot-run-00000000-00000000000000000000000000000000-0000000000000001");
        fs::create_dir(&foreign)?;
        fs::write(foreign.join(".lease"), b"owned by user")?;
        fs::write(foreign.join("test-gen1.exe"), b"keep this")?;

        #[cfg(any(windows, unix))]
        let linked_copy = {
            let target = root.path().join("link-target");
            fs::create_dir(&target)?;
            fs::write(target.join(LEASE), LEASE_MARKER)?;
            let target_copy = target.join("test-gen1.exe");
            fs::write(&target_copy, b"keep the link target")?;
            let link = root
                .path()
                .join("shenbot-run-00000000-00000000000000000000000000000000-0000000000000002");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link)?;
            #[cfg(windows)]
            if let Err(error) = std::os::windows::fs::symlink_dir(&target, &link) {
                use windows_sys::Win32::Foundation::ERROR_PRIVILEGE_NOT_HELD;

                if error.raw_os_error() != Some(ERROR_PRIVILEGE_NOT_HELD as i32) {
                    return Err(error);
                }
                eprintln!("当前 Windows 无符号链接权限，本次仅验证租约与外来文件");
            }
            target_copy
        };

        let second = ProcessRuntime::new(root.path())?;
        assert!(live.is_file(), "活动宿主的副本不应被清理");
        #[cfg(any(windows, unix))]
        assert_eq!(fs::read(linked_copy)?, b"keep the link target");
        assert!(!orphan.exists(), "已无租约的副本应在启动时回收");
        assert_eq!(fs::read(foreign.join("test-gen1.exe"))?, b"keep this");
        drop((runtime, prepared, second));
        assert!(!session.exists(), "最后一个副本持有者释放后应清理本会话");
        Ok(())
    }
}
