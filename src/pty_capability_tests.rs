use super::*;
use std::{
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

pub(in crate::pty) struct RuntimeFixture {
    path: PathBuf,
    _stage: enzui_core::tempstage::DirectoryStage,
}
impl RuntimeFixture {
    pub(in crate::pty) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Unix sockets need a short root even when the checkout or TMPDIR is long.
        let base = enzui_core::runtime_dir::resolve(
            None,
            enzui_core::runtime_dir::current_uid(),
            Path::new("/tmp"),
        )
        .unwrap();
        let parent = enzui_core::secure_dir::SecureRoot::open(base.path()).unwrap();
        let name = format!(
            "tc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let stage =
            enzui_core::tempstage::DirectoryStage::create(&parent, std::ffi::OsStr::new(&name))
                .unwrap();
        let fixture = Self {
            path: stage.path().to_owned(),
            _stage: stage,
        };
        for name in ["runtime", "home", "config", "state", "data", "cache"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(fixture.path.join(name))
                .unwrap();
        }
        fixture
    }
    pub(in crate::pty) fn runtime(&self) -> PathBuf {
        self.path.join("runtime")
    }
    pub(in crate::pty) fn configure(&self, cmd: &mut CommandBuilder) {
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.env("TERM", "xterm-256color");
        cmd.env("HOME", self.path.join("home"));
        for (key, name) in [
            ("XDG_RUNTIME_DIR", "runtime"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
        ] {
            cmd.env(key, self.path.join(name));
        }
        cmd.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-bus", self.runtime().display()),
        );
        cmd.cwd(&self.path);
    }
    fn command(&self) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/bash");
        cmd.args(["--noprofile", "--norc", "-c", "IFS= read -r -t 10 value"]);
        self.configure(&mut cmd);
        cmd
    }
}
fn pair() -> portable_pty::PtyPair {
    portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize::default())
        .unwrap()
}
struct ChildGuard {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    reaped: bool,
}
impl ChildGuard {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self {
            child,
            reaped: false,
        }
    }
    fn stop(&mut self) -> bool {
        if self.reaped {
            return true;
        }
        let _ = self.child.kill();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                self.reaped = true;
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn private_file(path: &Path) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    f.write_all(b"generated foreign sentinel").unwrap();
}
fn identity(path: &Path) -> (u64, u64) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}
