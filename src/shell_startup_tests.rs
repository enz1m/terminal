use crate::pinned_sessions::tests::Fixture;
use crate::pty;
use std::{
    collections::BTreeMap,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

fn installed() -> Vec<PathBuf> {
    let mut shells = Vec::new();
    for name in ["bash", "zsh", "fish"] {
        if let Some(path) = ["/usr/bin", "/bin"]
            .into_iter()
            .map(|dir| Path::new(dir).join(name))
            .find(|path| path.is_file())
        {
            shells.push(path);
        } else {
            eprintln!("SKIP native {name}: executable not installed");
        }
    }
    assert!(!shells.is_empty(), "no native shell exercised");
    shells
}
fn write_private(path: &Path, body: &str) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(body.as_bytes()).unwrap();
}
fn rc_files(f: &Fixture) -> BTreeMap<PathBuf, Vec<u8>> {
    [".bashrc", ".zshenv", ".zshrc", "config/fish/config.fish"]
        .into_iter()
        .map(|name| {
            let path = f.shell.join(name);
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}
/// The shared fixture seeds its session with demo history ("cargo build");
/// these gates assert what startup adds, so they begin from none.
fn startup_fixture() -> Fixture {
    let mut f = Fixture::new();
    f.app.sessions[0].in_app_history = Default::default();
    f
}

fn install(f: &mut Fixture, executable: &Path) -> Option<PathBuf> {
    install_with_bash_rc(
        f,
        executable,
        "PS1='native-shell> '; PROMPT_COMMAND=; HISTFILE=/dev/null\n",
    )
}
/// Starts the shell and waits for its prompt; the generated startup
/// directory (private, as are its files) that the prompt released.
fn install_with_bash_rc(f: &mut Fixture, executable: &Path, bash_rc: &str) -> Option<PathBuf> {
    f.app.ptys.remove(&f.pane);
    write_private(&f.shell.join(".bashrc"), bash_rc);
    write_private(&f.shell.join(".zshenv"), "\n");
    write_private(
        &f.shell.join(".zshrc"),
        "PROMPT='native-shell> '; RPROMPT=''; HISTFILE=/dev/null\n",
    );
    enzui_core::secure_dir::ensure_private_dir(&f.shell.join("config/fish")).unwrap();
    write_private(
        &f.shell.join("config/fish/config.fish"),
        "function fish_prompt; printf 'native-shell> '; end\nfunction fish_title; end\n",
    );
    let before = rc_files(f);
    let pty = pty::PtySession::spawn_native_history_probe(&f.pane, &f.shell, executable).unwrap();
    let stage = pty.generated_startup_path();
    if let Some(stage) = &stage {
        assert_eq!(
            std::fs::metadata(stage).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for entry in std::fs::read_dir(stage).unwrap() {
            assert_eq!(
                entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    f.app.install_pty(f.pane.clone(), pty);
    f.wait(|f| f.text().contains("native-shell>"));
    assert_eq!(rc_files(f), before);
    stage
}
fn fixture(executable: &Path) -> Fixture {
    fixture_with_stage(executable).0
}
/// A started shell, and the startup directory its first prompt removed.
fn fixture_with_stage(executable: &Path) -> (Fixture, PathBuf) {
    let mut f = startup_fixture();
    let stage = install(&mut f, executable).expect("a supported shell has startup files");
    (f, stage)
}
fn no_history_text(f: &Fixture, excluded: &str) {
    let history = &f.app.sessions[0].in_app_history;
    assert!(
        !history.is_empty(),
        "accepted-command positive control missing"
    );
    for i in 0..history.len() {
        assert!(!history.newest_first(i).unwrap().command.contains(excluded));
    }
    f.absent(excluded);
    f.no_file_bytes(excluded);
}
