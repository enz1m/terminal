use super::*;
use crate::emulator::Screen;
use std::{
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::DirBuilderExt,
    },
    sync::{Arc, atomic::AtomicU64},
};

fn screen() -> Screen {
    Screen::new(Arc::new(AtomicU64::new(0)))
}
fn report(path: &[u8]) -> Vec<u8> {
    format!(
        "\x1b]777;enzui-cwd;localhost;{}\x07",
        crate::session_store::hex(path)
    )
    .into_bytes()
}
fn accepted() -> Vec<u8> {
    format!(
        "\x1b]777;enzui-command;{}\x07\x1b]133;D;0\x07",
        crate::session_store::hex(b"echo accepted")
    )
    .into_bytes()
}
fn malformed() -> Vec<Vec<u8>> {
    [
        "enzui-cwd",
        "enzui-cwd;localhost",
        "enzui-cwd;localhost;",
        "enzui-cwd;localhost;2",
        "enzui-cwd;localhost;zz",
        "enzui-cwd;localhost;2f;extra",
        "enzui-cwd;remote;2f",
        "enzui-cwd;localhost;00",
        "enzui-cwd;localhost;2f00",
        "enzui-cwd;localhost;61",
        "enzui-cwd;localhost;２f",
    ]
    .into_iter()
    .map(|fields| format!("\x1b]777;{fields}\x07").into_bytes())
    .chain(std::iter::once(report(&[b'/'; MAX_PATH_BYTES + 1])))
    .collect()
}

#[test]
fn generated_cwd_roundtrips_unix_bytes_without_injecting_terminal_events() {
    let bytes = b"/space % # \xe7\x95\x8c\x07\x1b]2;injected\x07\xff tail ";
    let mut screen = screen();
    for chunk in report(bytes).chunks(3) {
        screen.process(chunk);
    }
    let effects = screen.take_effects();
    let cwd = effects.generated_cwd.as_ref().unwrap();
    assert_eq!(cwd.path().as_os_str().as_bytes(), bytes);
    assert_eq!(format!("{cwd:?}"), "ReportedCwd([redacted])");
    assert!(effects.cwd.is_none());
    assert!(effects.title.is_none());
    assert!(effects.commands_started.is_empty());
    assert!(effects.command_finished.is_none());
    assert_eq!(effects.bell_count, 0);
    assert!(screen.project("cwd").iter().all(|row| row.text.is_empty()));
    assert!(screen.take_effects().generated_cwd.is_none());
    screen.process(&accepted());
    let effects = screen.take_effects();
    assert_eq!(
        effects.commands_started.into_iter().collect::<Vec<_>>(),
        vec!["echo accepted".to_owned()]
    );
    assert_eq!(effects.command_finished, Some(0));
}

#[test]
fn malformed_cwd_reports_do_not_replace_the_last_valid_directory() {
    let mut screen = screen();
    screen.process(&report(b"/valid % path"));
    for bytes in malformed() {
        screen.process(&bytes);
    }
    let effects = screen.take_effects();
    assert_eq!(
        effects.generated_cwd.unwrap().path(),
        Path::new("/valid % path")
    );
    assert!(effects.cwd.is_none());
    assert!(effects.title.is_none());
    assert!(effects.commands_started.is_empty());
    assert_eq!(effects.bell_count, 0);
    for bytes in malformed() {
        screen.process(&bytes);
    }
    assert_eq!(screen.take_effects(), Default::default());
}

#[test]
fn cwd_limits_are_exact_and_invalid_reports_do_not_disable_acceptance() {
    assert_eq!(MAX_PATH_BYTES, 4096);
    let mut maximum = vec![b'x'; MAX_PATH_BYTES];
    maximum[0] = b'/';
    let mut screen = screen();
    screen.process(&report(&maximum));
    assert_eq!(
        screen
            .take_effects()
            .generated_cwd
            .unwrap()
            .path()
            .as_os_str()
            .as_bytes(),
        maximum
    );
    maximum.push(b'x');
    screen.process(&report(&maximum));
    assert!(screen.take_effects().generated_cwd.is_none());
    screen.process(&accepted());
    let effects = screen.take_effects();
    assert_eq!(effects.commands_started.len(), 1);
    assert_eq!(effects.command_finished, Some(0));
    assert!(effects.generated_cwd.is_none());
}

#[test]
fn child_osc_seven_keeps_its_existing_semantics_and_order() {
    let mut screen = screen();
    let child = b"\x1b]7;file://child.example/srv/app\x07";
    screen.process(&report(b"/generated"));
    screen.process(child);
    for bytes in malformed() {
        screen.process(&bytes);
    }
    let effects = screen.take_effects();
    assert_eq!(effects.cwd.as_deref(), Some("file://child.example/srv/app"));
    assert!(effects.generated_cwd.is_none());
    screen.process(child);
    screen.process(&report(b"/generated"));
    let effects = screen.take_effects();
    assert!(effects.cwd.is_none());
    assert_eq!(
        effects.generated_cwd.unwrap().path(),
        Path::new("/generated")
    );
    screen.process(b"\x1b]777;enzui-history-disabled\x07");
    screen.process(&report(b"/refused"));
    screen.process(child);
    let effects = screen.take_effects();
    assert!(effects.cwd.is_none() && effects.generated_cwd.is_none());
}

#[test]
fn local_launch_locator_preserves_bytes_without_control_diagnostics() {
    let f = crate::pinned_sessions::tests::Fixture::new();
    let path = f.shell.join(std::ffi::OsString::from_vec(
        b"space % # \xe7\x95\x8c\x07\x1b\xff tail ".to_vec(),
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let label = path_label(&path);
    assert!(label.is_ascii());
    assert!(!label.chars().any(char::is_control));
    assert_eq!(local_path(&label).unwrap(), path);
    let resolved = crate::pty::resolve_local_cwd(Some(&label));
    assert_eq!(resolved.effective, path);
    assert!(resolved.warning.is_none());
    std::fs::remove_dir(&path).unwrap();
    let resolved = crate::pty::resolve_local_cwd(Some(&label));
    let warning = resolved.warning.unwrap();
    assert!(!warning.chars().any(char::is_control));
    assert!(warning.contains("%07") && warning.contains("%1B") && warning.contains("%FF"));
    let literal = Path::new("/literal/%07");
    assert_eq!(path_label(literal), "/literal/%07");
    assert!(local_path(&path_label(literal)).is_none());
}
