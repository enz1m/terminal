use enzui_core::{
    runtime_dir,
    secure_dir::{SecureRoot, open_file_identity},
    tempstage::unique_temp_file_name,
};
use portable_pty::CommandBuilder;
use rustix::fs::{Mode, OFlags};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, Metadata},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const HANDSHAKE_BUDGET: Duration = Duration::from_millis(500);
const CHILD_SCRIPT: &str = include_str!("pty_capability.sh");

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "PTY capability identity is unavailable or changed",
    )
}

struct CapabilityName {
    root: SecureRoot,
    name: Option<OsString>,
    identity: (u64, u64),
    // Keep the inode allocated even if the name is removed before the usable FIFO open.
    _anchor: File,
}

impl CapabilityName {
    fn verify(&self) -> io::Result<()> {
        self.root.revalidate().map_err(io::Error::other)?;
        let name = self.name.as_ref().ok_or_else(refused)?;
        if self.root.entry_identity(name)? != self.identity {
            return Err(refused());
        }
        Ok(())
    }

    fn unlink(&mut self) -> io::Result<bool> {
        let Some(name) = &self.name else {
            return Ok(true);
        };
        let removed = self.root.unlink_entry_if_identity(name, self.identity)?;
        if removed {
            self.name = None;
        }
        Ok(removed)
    }
}

impl Drop for CapabilityName {
    fn drop(&mut self) {
        let _ = self.unlink();
    }
}

pub(super) struct PtyCapability {
    device: u64,
    inode: u64,
    // Retained after unlink so inode recycling cannot authorize an unrelated process.
    handle: Arc<File>,
    pending: Option<CapabilityName>,
}

impl PtyCapability {
    pub(super) fn create(cmd: &CommandBuilder) -> io::Result<Self> {
        let runtime = runtime_dir::resolve(
            cmd.get_env("XDG_RUNTIME_DIR").map(OsStr::to_owned),
            runtime_dir::current_uid(),
            Path::new("/tmp"),
        )
        .map_err(io::Error::other)?;
        if runtime.is_refusal() {
            return Err(refused());
        }
        Self::create_at(runtime.join(unique_temp_file_name(OsStr::new("pty-capability"))))
    }

    fn create_at(path: PathBuf) -> io::Result<Self> {
        Self::create_at_with(
            path,
            |_| Ok(()),
            SecureRoot::open_entry_read,
            File::metadata,
        )
    }

    fn create_at_with<A, O, S>(
        path: PathBuf,
        after_created: A,
        open: O,
        stat: S,
    ) -> io::Result<Self>
    where
        A: FnOnce(&CapabilityName) -> io::Result<()>,
        O: FnOnce(&SecureRoot, &OsStr) -> io::Result<File>,
        S: FnOnce(&File) -> io::Result<Metadata>,
    {
        let parent = path.parent().ok_or_else(refused)?;
        let name = path.file_name().ok_or_else(refused)?.to_owned();
        let root = SecureRoot::open(parent).map_err(io::Error::other)?;
        // SecureRoot's fd is private; specialized *at calls need an identity-matched
        // directory descriptor.
        let directory = File::from(rustix::fs::open(
            parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let metadata = directory.metadata()?;
        if open_file_identity(&directory)? != root.identity()
            || metadata.mode() & 0o077 != 0
            || Some(metadata.uid()) != runtime_dir::current_uid()
        {
            return Err(refused());
        }
        root.revalidate().map_err(io::Error::other)?;
        rustix::fs::mkfifoat(&directory, &name, Mode::RUSR | Mode::WUSR)?;
        let identity = root.entry_identity(&name)?;
        let anchor = File::from(rustix::fs::openat(
            &directory,
            &name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        if open_file_identity(&anchor)? != identity || !anchor.metadata()?.file_type().is_fifo() {
            return Err(refused());
        }
        let pending = CapabilityName {
            root,
            name: Some(name.clone()),
            identity,
            _anchor: anchor,
        };
        after_created(&pending)?;
        pending.verify()?;
        let handle = open(&pending.root, &name)?;
        let metadata = stat(&handle)?;
        if !metadata.file_type().is_fifo()
            || (metadata.dev(), metadata.ino()) != identity
            || Some(metadata.uid()) != runtime_dir::current_uid()
            || metadata.mode() & 0o077 != 0
        {
            return Err(refused());
        }
        handle.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        pending.verify()?;
        Ok(Self {
            device: identity.0,
            inode: identity.1,
            handle: Arc::new(handle),
            pending: Some(pending),
        })
    }

    #[cfg(test)]
    pub(super) fn create_for_test(
        path: PathBuf,
        failure: Option<super::PtySetupFailure>,
        visible: Option<&std::sync::atomic::AtomicBool>,
    ) -> io::Result<Self> {
        use super::PtySetupFailure;
        if failure == Some(PtySetupFailure::Mkfifo) {
            return Err(io::Error::other("injected mkfifo failure"));
        }
        Self::create_at_with(
            path,
            |_| {
                if let Some(visible) = visible {
                    visible.store(true, std::sync::atomic::Ordering::Release);
                }
                if failure == Some(PtySetupFailure::Open) {
                    return Err(io::Error::other("injected FIFO open failure"));
                }
                Ok(())
            },
            SecureRoot::open_entry_read,
            |file| {
                if failure == Some(PtySetupFailure::Metadata) {
                    return Err(io::Error::other("injected FIFO metadata failure"));
                }
                file.metadata()
            },
        )
    }

    pub(super) fn retain_handoff(&self) -> Arc<File> {
        Arc::clone(&self.handle)
    }

    pub(super) fn handoff_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.handle.as_raw_fd()
        ))
    }

    pub(super) fn seal(mut self, child: Option<u32>) -> Option<Self> {
        let child = child?;
        self.pending.as_ref()?.verify().ok()?;
        let mut ready = [filedescriptor::pollfd {
            fd: self.handle.as_raw_fd(),
            events: filedescriptor::POLLIN,
            revents: 0,
        }];
        if !matches!(filedescriptor::poll(&mut ready, Some(HANDSHAKE_BUDGET)), Ok(count) if count > 0)
            || ready[0].revents & filedescriptor::POLLIN == 0
        {
            return None;
        }
        let mut byte = [0];
        let mut reader = self.handle.as_ref();
        if reader.read(&mut byte).ok()? != 1 || byte[0] != b'.' {
            return None;
        }
        // A receipt alone cannot establish inheritance: the unreaped child must
        // hold the created FIFO on fd 9.
        if !self.child_descriptor(child, OsStr::new("9")) {
            return None;
        }
        self.pending.as_ref()?.verify().ok()?;
        if !self.pending.as_mut()?.unlink().ok()? {
            return None;
        }
        self.pending.take();
        Some(self)
    }

    fn names_capability(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|metadata| {
            metadata.file_type().is_fifo()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        })
    }

    fn child_descriptor(&self, pid: u32, descriptor: &OsStr) -> bool {
        let path = PathBuf::from(format!("/proc/{pid}/fd")).join(descriptor);
        if !self.names_capability(&path) {
            return false;
        }
        let info = PathBuf::from(format!("/proc/{pid}/fdinfo")).join(descriptor);
        let Ok(file) = File::open(info) else {
            return false;
        };
        let mut text = String::new();
        if file.take(4097).read_to_string(&mut text).is_err() || text.len() > 4096 {
            return false;
        }
        let flags = text.lines().find_map(|line| {
            line.strip_prefix("flags:")
                .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
        });
        // A fork may temporarily inherit the parent's read-only anchor; only the child's reopened read/write end grants ownership.
        flags.is_some_and(|flags| flags & OFlags::ACCMODE.bits() == OFlags::RDWR.bits())
            && self.names_capability(&path)
    }

    pub(super) fn is_held_by(&self, pid: u32) -> bool {
        std::fs::read_dir(format!("/proc/{pid}/fd")).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| self.child_descriptor(pid, &entry.file_name()))
        })
    }
}

pub(super) fn wrap_with_capability(mut cmd: CommandBuilder, handoff: &Path) -> CommandBuilder {
    let mut argv = vec![
        "/bin/sh".into(),
        "-c".into(),
        CHILD_SCRIPT.into(),
        "enzui-pty-capability".into(),
        handoff.as_os_str().to_owned(),
    ];
    argv.append(cmd.get_argv_mut());
    *cmd.get_argv_mut() = argv;
    cmd
}

#[cfg(test)]
#[path = "pty_capability_tests.rs"]
pub(super) mod tests;
