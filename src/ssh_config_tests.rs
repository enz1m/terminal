use super::*;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "terminal-ssh-config-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join(".ssh/config.d")).unwrap();
        Self(root)
    }
    fn write(&self, relative: &str, body: &str) {
        fs::write(self.0.join(relative), body).unwrap();
    }
    fn load(&self) -> Vec<OpenSshHost> {
        load_user_config(&self.0.join(".ssh/config"), &self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn aliases_include_plain_and_included_but_exclude_wildcard_match_and_known_hosts() {
    let f = Fixture::new();
    f.write(
        ".ssh/config",
        concat!(
            "Host plain\n  HostName plain.example\n",
            "Host *\n  User fallback\n",
            "Match host matched\n  User forbidden\n",
            "Include config.d/*.conf\n",
        ),
    );
    f.write(
        ".ssh/config.d/team.conf",
        "Host included\n  HostName included.example\n",
    );
    // Positive control for the absence claim: a value that would become an
    // alias if known_hosts were (incorrectly) treated as bookmarks.
    f.write(".ssh/known_hosts", "leaked.example ssh-ed25519 AAAATEST\n");
    let hosts = f.load();
    assert_eq!(
        hosts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
        vec!["plain", "included"]
    );
    assert!(!hosts.iter().any(|h| h.alias == "leaked.example"));
}

#[test]
fn scalar_options_use_openssh_first_obtained_value_precedence() {
    let f = Fixture::new();
    f.write(".ssh/config", concat!(
        "Host exact\n",
        "  HostName first.example\n  User first-user\n  Port 2201\n  IdentityFile ~/.ssh/first\n",
        "Host exact\n",
        "  HostName second.example\n  User second-user\n  Port 2202\n  IdentityFile ~/.ssh/second\n",
        "Host *\n  User wildcard-user\n  Port 22\n",
    ));
    let host = f.load().into_iter().find(|h| h.alias == "exact").unwrap();
    assert_eq!(host.hostname, "first.example");
    assert_eq!(host.user, "first-user");
    assert_eq!(host.port, 2201);
    assert_eq!(host.identity_file.as_deref(), Some("~/.ssh/first"));
}
