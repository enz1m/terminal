use enzui_core::{runtime_dir, secure_dir::SecureRoot, tempstage::DirectoryStage};
use portable_pty::CommandBuilder;
use std::{
    ffi::OsStr,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub(crate) const UNAVAILABLE: &str =
    "Command history unavailable: shell integration startup failed.";
pub(crate) const UNSUPPORTED: &str =
    "Command history unavailable: unsupported shell or invocation.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shell {
    Bash,
    Zsh,
    Fish,
}

pub(crate) fn detect(cmd: &CommandBuilder) -> Option<(Shell, PathBuf)> {
    let argv = cmd.get_argv();
    let program = Path::new(argv.first()?);
    // sh/ksh and restricted aliases change shell semantics even when they resolve to Bash or zsh.
    let name = program.file_name()?.to_str()?;
    if name.starts_with('-')
        || matches!(
            name,
            "sh" | "ksh" | "rbash" | "rzsh" | "rsh" | "dash" | "ash" | "csh" | "tcsh"
        )
        || argv.iter().skip(1).any(|arg| arg != "-i")
    {
        return None;
    }
    let executable = if program.is_absolute() {
        program.to_owned()
    } else {
        crate::pty::resolve_executable_in(program.to_str()?, cmd.get_env("PATH")?)?
    };
    let executable = executable.canonicalize().ok()?;
    let metadata = executable.metadata().ok()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return None;
    }
    let shell = match executable.file_name()?.to_str()? {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        _ => return None,
    };
    Some((shell, executable))
}

const STAGE: &str = "terminal-shell";

/// Removes the startup directories of terminals that stopped before their
/// shells started (a pane releases its own at the first prompt):
/// `terminal-shell.tmp-<pid>-…` whose process is gone. Links and files are
/// left alone.
pub(crate) fn sweep_abandoned(runtime: &Path) {
    let Ok(entries) = std::fs::read_dir(runtime) else {
        return;
    };
    for entry in entries.filter_map(Result::ok).take(16384) {
        let Some(pid) = stage_owner(&entry.file_name()) else {
            continue;
        };
        if Path::new("/proc").join(pid.to_string()).exists() {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The process that created a startup directory, from its name.
fn stage_owner(name: &OsStr) -> Option<u32> {
    let rest = name.to_str()?.strip_prefix(STAGE)?.strip_prefix(".tmp-")?;
    rest.split_once('-')?.0.parse().ok()
}

pub(crate) struct Startup {
    _stage: DirectoryStage,
}
impl Startup {
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        self._stage.path()
    }
}

pub(crate) fn prepare(cmd: &mut CommandBuilder) -> io::Result<Option<Startup>> {
    let Some((shell, executable)) = detect(cmd) else {
        return Ok(None);
    };
    let runtime = match cmd.get_env("XDG_RUNTIME_DIR") {
        Some(path) => PathBuf::from(path),
        None => runtime_dir::runtime_dir()
            .map_err(io::Error::other)?
            .path()
            .to_owned(),
    };
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(|| sweep_abandoned(&runtime));
    let parent = SecureRoot::open(&runtime).map_err(io::Error::other)?;
    let name = enzui_core::tempstage::unique_temp_file_name(OsStr::new(STAGE));
    let stage = DirectoryStage::create(&parent, &name)?;
    let write = |name: &str, body: &str| -> io::Result<()> {
        stage
            .root()
            .create_entry_private(OsStr::new(name))?
            .write_all(body.as_bytes())
    };
    match shell {
        Shell::Bash => {
            let array = format!(
                "PROMPT_COMMAND=({} \"${{PROMPT_COMMAND[@]}}\")",
                crate::pty::shell_single_quote(super::BASH_PROMPT_COMMAND),
            );
            let body = include_str!("shell_startup.bash")
                .replace(
                    "@ENZUI_PROMPT_ARRAY@",
                    &crate::pty::shell_single_quote(&array),
                )
                .replace("@ENZUI_HISTORY_DISABLED@", super::HISTORY_DISABLED);
            write("bashrc", &body)?;
        }
        Shell::Zsh => {
            write(".zshenv", ZSH_ENV)?;
            write(".zshrc", ZSH_RC)?;
            write("integration.zsh", include_str!("shell_history.zsh"))?;
        }
        Shell::Fish => write("integration.fish", include_str!("shell_history.fish"))?,
    }
    cmd.get_argv_mut().truncate(1);
    cmd.get_argv_mut()[0] = executable.into_os_string();
    match shell {
        Shell::Bash => {
            cmd.arg("--rcfile");
            cmd.arg(stage.path().join("bashrc"));
            cmd.arg("-i");
        }
        Shell::Zsh => {
            let original = cmd
                .get_env("ZDOTDIR")
                .or_else(|| cmd.get_env("HOME"))
                .unwrap_or(OsStr::new(""))
                .to_owned();
            cmd.env("ENZUI_ORIGINAL_ZDOTDIR", original);
            cmd.env("ZDOTDIR", stage.path());
            cmd.arg("-i");
        }
        Shell::Fish => {
            cmd.env("ENZUI_SHELL_STARTUP", stage.path());
            cmd.args(["--interactive", "--init-command", r#"builtin source "$ENZUI_SHELL_STARTUP/integration.fish"; or builtin printf '\033]777;enzui-history-disabled\007'"#]);
        }
    }
    Ok(Some(Startup { _stage: stage }))
}

const ZSH_ENV: &str = r#"builtin typeset -g __enzui_stage=$ZDOTDIR
ZDOTDIR=$ENZUI_ORIGINAL_ZDOTDIR
builtin unset ENZUI_ORIGINAL_ZDOTDIR
if [[ -r $ZDOTDIR/.zshenv ]]; then
    builtin source "$ZDOTDIR/.zshenv"
fi
builtin typeset -g __enzui_user_zdotdir=${ZDOTDIR:-$HOME}
ZDOTDIR=$__enzui_stage
"#;

const ZSH_RC: &str = r#"ZDOTDIR=$__enzui_user_zdotdir
if [[ -r $ZDOTDIR/.zshrc ]]; then
    builtin source "$ZDOTDIR/.zshrc"
fi
if ! builtin printf ''; then
    builtin print -rn -- $'\033]777;enzui-history-disabled\007'
else
    builtin source "$__enzui_stage/integration.zsh" || builtin printf '\033]777;enzui-history-disabled\007'
fi
builtin unset __enzui_stage __enzui_user_zdotdir
"#;
