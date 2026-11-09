//! The git branch a local directory is on, for the sidebar's third line.
//!
//! Read from the repository's own files (`.git/HEAD`, or the `gitdir:` a
//! worktree's `.git` file points at), not by running git: a lookup is a few
//! small reads, made off the UI thread through the blocking dispatcher.

use std::path::{Path, PathBuf};

/// How many parent directories a lookup climbs looking for `.git`.
const MAX_DEPTH: usize = 64;

/// Bytes read from `HEAD` or a `.git` file; real ones are a single line.
const MAX_FILE_BYTES: u64 = 4096;

/// The branch checked out in the repository containing `dir`, or the short
/// commit id of a detached `HEAD`; `None` outside a repository.
pub(crate) fn branch_at(dir: &Path) -> Option<String> {
    let git_dir = dir.ancestors().take(MAX_DEPTH).find_map(git_dir_of)?;
    let head = read_small(&git_dir.join("HEAD"))?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!name.is_empty() && !name.chars().any(char::is_control)).then(|| name.to_owned());
    }
    head.bytes()
        .all(|byte| byte.is_ascii_hexdigit())
        .then(|| head.get(..7).map(str::to_owned))
        .flatten()
}

/// The git directory `dir` holds: its `.git` directory, or the one a `.git`
/// file (a worktree or submodule) names.
fn git_dir_of(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    let metadata = std::fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git);
    }
    let pointer = read_small(&dot_git)?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_owned()
    } else {
        dir.join(target)
    })
}

fn read_small(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

impl crate::App {
    /// Starts the branch lookups queued since the last update, off the UI
    /// thread. A lookup refused because the dispatcher is busy is dropped:
    /// the next prompt asks again.
    pub(crate) fn look_up_git_branches(&mut self) -> iced::Task<crate::Message> {
        if self.git_lookups_due.is_empty() {
            return iced::Task::none();
        }
        let mut latest: std::collections::HashMap<String, PathBuf> =
            std::collections::HashMap::new();
        for (pane, dir) in self.git_lookups_due.drain(..) {
            latest.insert(pane, dir);
        }
        iced::Task::batch(latest.into_iter().map(|(pane, dir)| {
            let lookup = self.blocking.submit_fifo(move |_| branch_at(&dir));
            iced::Task::perform(
                async move { lookup.await.into_result().ok().flatten() },
                move |branch| crate::Message::GitBranch(pane, branch),
            )
        }))
    }

    /// Records the branch `pane`'s directory is on, redrawing the sidebar
    /// when it changed. A pane that has closed since is forgotten.
    pub(crate) fn set_git_branch(&mut self, pane: String, branch: Option<String>) {
        let open = self
            .sessions
            .iter()
            .any(|session| session.panes.iter().any(|p| p.id == pane));
        let changed = match branch.filter(|_| open) {
            Some(branch) => self.git_branches.insert(pane, branch.clone()) != Some(branch),
            None => self.git_branches.remove(&pane).is_some(),
        };
        if changed {
            self.invalidate_sidebar();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory, removed when dropped.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("terminal-git-branch-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn repo(head: &str) -> Scratch {
        let root = Scratch::new();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        std::fs::write(root.path().join(".git/HEAD"), head).unwrap();
        root
    }
}
