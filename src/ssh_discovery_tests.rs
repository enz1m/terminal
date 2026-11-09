use super::*;
use iced::futures::Stream;
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};
fn host(name: &str) -> OpenSshHost {
    OpenSshHost {
        config_fingerprint: 7,
        alias: name.into(),
        hostname: format!("{name}.invalid"),
        user: "user".into(),
        port: 22,
        identity_file: None,
    }
}
fn next<T>(stream: &mut (impl Stream<Item = T> + Unpin)) -> T {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match Pin::new(&mut *stream).poll_next(&mut context) {
            Poll::Ready(Some(item)) => return item,
            Poll::Ready(None) => panic!("discovery stream ended"),
            Poll::Pending => {
                assert!(Instant::now() < deadline, "discovery completion deadline");
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
fn next_discovery(
    stream: &mut (impl Stream<Item = iced_test::runtime::Action<crate::Message>> + Unpin),
) -> crate::Message {
    for _ in 0..16 {
        if let iced_test::runtime::Action::Output(
            message @ crate::Message::OpenSshDiscoveryFinished(_),
        ) = next(stream)
        {
            return message;
        }
    }
    panic!("discovery completion was not routed");
}
struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[cfg(target_os = "linux")]
struct Root(std::path::PathBuf);
#[cfg(target_os = "linux")]
impl Root {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".openssh-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let value = Self(root);
        for name in [".ssh", "config", "cache", "state", "data", "runtime"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(value.0.join(name))
                .unwrap();
        }
        value
    }
}
#[cfg(target_os = "linux")]
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
