use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) fn parse(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<String>, &'static str> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut args = args.into_iter();
    let mut directory = None;
    while let Some(arg) = args.next() {
        let path = if arg == "--working-directory" {
            args.next().ok_or("--working-directory needs a directory")?
        } else if let Some(value) = arg.as_bytes().strip_prefix(b"--working-directory=") {
            OsString::from_vec(value.to_vec())
        } else {
            continue;
        };
        if directory.is_some() || path.is_empty() {
            return Err("provide one nonempty --working-directory");
        }
        let path = PathBuf::from(path)
            .canonicalize()
            .map_err(|_| "working directory is unavailable")?;
        if !path.is_dir() {
            return Err("working directory is not a directory");
        }
        directory = Some(crate::shell_history::cwd::path_label(&path));
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "terminal-launch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            for name in ["config", "state", "data", "cache", "runtime"] {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(path.join(name))
                    .unwrap();
            }
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}
