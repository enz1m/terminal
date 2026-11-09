use super::*;
#[cfg(target_os = "linux")]
use rustix::fs::{Mode, OFlags};
use std::{
    ffi::{CStr, OsStr},
    fs::File,
    io::Read,
};

pub(super) struct Input {
    file: File,
    parent: File,
    pub body: String,
    pub identity: (u64, u64),
    length: u64,
    modified: std::time::SystemTime,
}
impl Input {
    pub(super) fn verify(&self, path: &Path, budget: &mut Budget<'_>) -> Result<()> {
        let (parent, _) = open_parent(path, budget)?;
        if identity(&parent)? != identity(&self.parent)? {
            return Err(OpenSshConfigError::invalid(
                "OpenSSH source parent changed.",
            ));
        }
        enzui_core::tempstage::verify_path_names_file(&self.file, path)
            .map_err(|_| OpenSshConfigError::invalid("OpenSSH source identity changed."))?;
        let metadata = self
            .file
            .metadata()
            .map_err(|e| OpenSshConfigError::io(path, e))?;
        if metadata.len() != self.length
            || metadata
                .modified()
                .map_err(|e| OpenSshConfigError::io(path, e))?
                != self.modified
        {
            return Err(OpenSshConfigError::invalid(
                "OpenSSH source metadata changed.",
            ));
        }
        budget.check()
    }
}
fn identity(file: &File) -> Result<(u64, u64)> {
    enzui_core::secure_dir::open_file_identity(file)
        .map_err(|_| OpenSshConfigError::invalid("OpenSSH source identity unavailable."))
}
#[cfg(target_os = "linux")]
fn name<'a>(part: &OsStr, buffer: &'a mut [u8; 4097]) -> Result<&'a CStr> {
    let bytes = part.as_encoded_bytes();
    if bytes.len() >= buffer.len() {
        return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
    }
    buffer[..bytes.len()].copy_from_slice(bytes);
    buffer[bytes.len()] = 0;
    CStr::from_bytes_with_nul(&buffer[..=bytes.len()])
        .map_err(|_| OpenSshConfigError::invalid("OpenSSH source path contains NUL."))
}
#[cfg(target_os = "linux")]
fn walk_directory(path: &Path, budget: &mut Budget<'_>) -> Result<File> {
    use std::path::Component;
    budget.check()?;
    if path.as_os_str().as_encoded_bytes().len() > budget.limits.string_bytes {
        return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
    }
    let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut dir = File::from(
        rustix::fs::open(
            if path.is_absolute() { c"/" } else { c"." },
            flags | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .map_err(|e| OpenSshConfigError::io(path, e.into()))?,
    );
    let mut buffer = [0; 4097];
    for component in path.components() {
        if matches!(component, Component::RootDir | Component::CurDir) {
            continue;
        }
        budget.check()?;
        dir = File::from(
            rustix::fs::openat(
                &dir,
                name(component.as_os_str(), &mut buffer)?,
                flags,
                Mode::empty(),
            )
            .map_err(|e| OpenSshConfigError::io(path, e.into()))?,
        );
        let metadata = dir
            .metadata()
            .map_err(|e| OpenSshConfigError::io(path, e))?;
        if metadata.file_type().is_symlink() {
            return Err(OpenSshConfigError::invalid(
                "OpenSSH source ancestor is a symlink.",
            ));
        }
        if !metadata.is_dir() {
            return Err(OpenSshConfigError::io(
                path,
                std::io::ErrorKind::NotADirectory.into(),
            ));
        }
    }
    Ok(dir)
}
#[cfg(target_os = "linux")]
fn open_parent<'p>(path: &'p Path, budget: &mut Budget<'_>) -> Result<(File, &'p OsStr)> {
    if path.as_os_str().as_encoded_bytes().len() > budget.limits.string_bytes {
        return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
    }
    let leaf = path
        .file_name()
        .ok_or_else(|| OpenSshConfigError::invalid("OpenSSH source has no filename."))?;
    Ok((
        walk_directory(path.parent().unwrap_or(Path::new(".")), budget)?,
        leaf,
    ))
}
#[cfg(not(target_os = "linux"))]
fn open_parent<'p>(_: &'p Path, _: &mut Budget<'_>) -> Result<(File, &'p OsStr)> {
    Err(OpenSshConfigError::unavailable())
}
#[cfg(target_os = "linux")]
fn open_regular(
    path: &Path,
    budget: &mut Budget<'_>,
    after_parent: impl FnOnce(),
) -> Result<(File, File)> {
    let (parent, leaf) = open_parent(path, budget)?;
    after_parent();
    let mut buffer = [0; 4097];
    let leaf = name(leaf, &mut buffer)?;
    // Both opens address the held parent; re-resolving the pathname could switch
    // ancestors between regular-inode admission and the data open.
    let anchor = File::from(
        rustix::fs::openat(
            &parent,
            leaf,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| OpenSshConfigError::io(path, e.into()))?,
    );
    let before = anchor
        .metadata()
        .map_err(|e| OpenSshConfigError::io(path, e))?;
    if !before.is_file() {
        return Err(OpenSshConfigError::invalid(
            "OpenSSH source is not a regular file.",
        ));
    }
    budget.check()?;
    let file = File::from(
        rustix::fs::openat(
            &parent,
            leaf,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| OpenSshConfigError::io(path, e.into()))?,
    );
    let after = file
        .metadata()
        .map_err(|e| OpenSshConfigError::io(path, e))?;
    if !after.is_file() || identity(&anchor)? != identity(&file)? {
        return Err(OpenSshConfigError::invalid(
            "OpenSSH source identity changed before read.",
        ));
    }
    Ok((file, parent))
}
#[cfg(not(target_os = "linux"))]
fn open_regular(_: &Path, _: &mut Budget<'_>, _: impl FnOnce()) -> Result<(File, File)> {
    Err(OpenSshConfigError::unavailable())
}

pub(super) fn read(path: &Path, active: &[(u64, u64)], budget: &mut Budget<'_>) -> Result<Input> {
    read_with(path, active, budget, || {}, |_| {})
}
fn read_with(
    path: &Path,
    active: &[(u64, u64)],
    budget: &mut Budget<'_>,
    after_parent: impl FnOnce(),
    after_admission: impl FnOnce(&File),
) -> Result<Input> {
    budget.check()?;
    let (mut file, parent) = open_regular(path, budget, after_parent)?;
    let identity = identity(&file)?;
    if active.contains(&identity) {
        return Err(OpenSshConfigError::invalid("OpenSSH Include cycle."));
    }
    let metadata = file
        .metadata()
        .map_err(|e| OpenSshConfigError::io(path, e))?;
    let length = usize::try_from(metadata.len())
        .map_err(|_| OpenSshConfigError::exceeded(BudgetKind::FileBytes))?;
    if length > budget.limits.file_bytes {
        return Err(OpenSshConfigError::exceeded(BudgetKind::FileBytes));
    }
    budget.charge(BudgetKind::TotalBytes, length)?;
    let probe_limit = length
        .checked_add(1)
        .ok_or_else(|| OpenSshConfigError::exceeded(BudgetKind::FileBytes))?;
    budget.storage::<u8>(probe_limit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(probe_limit)
        .map_err(|_| OpenSshConfigError::unavailable())?;
    after_admission(&file);
    let mut chunk = [0_u8; 8192];
    while bytes.len() < probe_limit {
        budget.check()?;
        let count = chunk.len().min(probe_limit - bytes.len());
        match file.read(&mut chunk[..count]) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(OpenSshConfigError::io(path, e)),
        }
    }
    if bytes.len() != length {
        return Err(OpenSshConfigError::invalid(
            "OpenSSH source changed during admitted read.",
        ));
    }
    let body = String::from_utf8(bytes)
        .map_err(|_| OpenSshConfigError::invalid_at(path, "OpenSSH source is not UTF-8."))?;
    let input = Input {
        file,
        parent,
        body,
        identity,
        length: metadata.len(),
        modified: metadata
            .modified()
            .map_err(|e| OpenSshConfigError::io(path, e))?,
    };
    input.verify(path, budget)?;
    Ok(input)
}

#[cfg(target_os = "linux")]
fn directory(path: &Path, budget: &mut Budget<'_>) -> Result<File> {
    let parent = walk_directory(path, budget)?;
    Ok(File::from(
        rustix::fs::openat(
            &parent,
            c".",
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| OpenSshConfigError::io(path, e.into()))?,
    ))
}
#[cfg(target_os = "linux")]
fn children(
    parent: &Path,
    component: &str,
    next: &mut Vec<PathBuf>,
    budget: &mut Budget<'_>,
) -> Result<()> {
    let held = match directory(parent, budget) {
        Ok(file) => file,
        Err(error)
            if matches!(
                error.kind(),
                ConfigFailureKind::Missing | ConfigFailureKind::NotDirectory
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let mut buffer = [std::mem::MaybeUninit::uninit(); 8192];
    let mut entries = rustix::fs::RawDir::new(&held, &mut buffer);
    loop {
        budget.check()?;
        let Some(entry) = entries.next() else {
            break;
        };
        let entry = entry.map_err(|e| OpenSshConfigError::io(parent, e.into()))?;
        budget.charge(BudgetKind::GlobCandidates, 1)?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::str::from_utf8(bytes)
            .map_err(|_| OpenSshConfigError::invalid("OpenSSH Include entry is not UTF-8."))?;
        budget.text(name)?;
        if name.starts_with('.') && !component.starts_with('.') {
            continue;
        }
        if resolve::glob(component, name, budget)? {
            let path = budget.join(parent, OsStr::new(name))?;
            budget.push(next, path)?;
        }
    }
    enzui_core::tempstage::verify_path_names_file(&held, parent)
        .map_err(|_| OpenSshConfigError::invalid("OpenSSH Include directory changed."))?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn children(_: &Path, _: &str, _: &mut Vec<PathBuf>, _: &mut Budget<'_>) -> Result<()> {
    Err(OpenSshConfigError::unavailable())
}

pub(super) fn expand(
    token: &str,
    home: &Path,
    root: &Path,
    budget: &mut Budget<'_>,
) -> Result<Vec<PathBuf>> {
    let expanded = if token == "~" {
        budget.path(home)?
    } else if let Some(rest) = token.strip_prefix("~/") {
        budget.join(home, OsStr::new(rest))?
    } else if Path::new(token).is_absolute() {
        budget.path(Path::new(token))?
    } else {
        budget.join(root, OsStr::new(token))?
    };
    let text = expanded
        .to_str()
        .ok_or_else(|| OpenSshConfigError::invalid("OpenSSH Include path is not UTF-8."))?;
    let mut candidates = Vec::new();
    if !resolve::has_meta(text) {
        budget.charge(BudgetKind::GlobCandidates, 1)?;
        budget.push(&mut candidates, expanded)?;
        return Ok(candidates);
    }
    let start = budget.path(if expanded.is_absolute() {
        Path::new("/")
    } else {
        Path::new(".")
    })?;
    budget.push(&mut candidates, start)?;
    for component in text.split('/').filter(|part| !part.is_empty()) {
        let mut next = Vec::new();
        for parent in candidates {
            budget.check()?;
            if resolve::has_meta(component) {
                children(&parent, component, &mut next, budget)?;
            } else {
                budget.charge(BudgetKind::GlobCandidates, 1)?;
                let child = budget.join(&parent, OsStr::new(component))?;
                budget.push(&mut next, child)?;
            }
        }
        next.sort_unstable();
        candidates = next;
    }
    Ok(candidates)
}

#[cfg(test)]
pub(super) fn read_probe(
    path: &Path,
    budget: &mut Budget<'_>,
    hook: impl FnOnce(&File),
) -> Result<Input> {
    read_with(path, &[], budget, || {}, hook)
}
#[cfg(test)]
pub(super) fn read_parent_probe(
    path: &Path,
    budget: &mut Budget<'_>,
    parent: impl FnOnce(),
    admitted: impl FnOnce(&File),
) -> Result<Input> {
    read_with(path, &[], budget, parent, admitted)
}
