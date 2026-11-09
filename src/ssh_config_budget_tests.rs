use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".openssh-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        builder.create(path.join(".ssh")).unwrap();
        Self(path)
    }
    fn config(&self) -> PathBuf {
        self.0.join(".ssh/config")
    }
    fn write(&self, name: &str, body: &str) {
        fs::write(self.0.join(".ssh").join(name), body).unwrap();
    }
    fn load(&self, limits: Limits) -> Result<(Vec<OpenSshHost>, Stats)> {
        load_with(&self.config(), &self.0, limits, &|| true)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn refusal(result: Result<(Vec<OpenSshHost>, Stats)>, kind: BudgetKind, path: &Path) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), ConfigFailureKind::Budget);
    assert_eq!(
        error.diagnostic(),
        format!("OpenSSH discovery budget exceeded: {kind:?}.")
    );
    assert!(!error.diagnostic().contains(&path.display().to_string()));
    assert!(error.user_summary().contains("unavailable"));
}

#[test]
fn every_admission_family_refuses_plus_one_before_partial_results() {
    let f = Fixture::new();
    let limits = Limits::default();
    assert_eq!(
        (
            limits.file_bytes,
            limits.total_bytes,
            limits.depth,
            limits.files
        ),
        (256 * 1024, 2 * 1024 * 1024, 16, 128)
    );
    assert_eq!(
        (
            limits.aliases,
            limits.tokens,
            limits.directives,
            limits.glob_candidates
        ),
        (1024, 32768, 16384, 4096)
    );
    assert_eq!(
        (limits.string_bytes, limits.retained_bytes),
        (4096, 4 * 1024 * 1024)
    );
    let body = format!(
        "Host alpha beta\nUser {}\n",
        "u".repeat(limits.string_bytes)
    );
    f.write("config", &body);
    let (hosts, stats) = f.load(limits).unwrap();
    assert_eq!(hosts.len(), 2);
    assert!(stats.retained > 8192 && stats.work > 0);
    type Set = fn(&mut Limits, usize);
    let cases: [(BudgetKind, usize, Set); 9] = [
        (BudgetKind::FileBytes, body.len(), |l, n| l.file_bytes = n),
        (BudgetKind::TotalBytes, body.len(), |l, n| l.total_bytes = n),
        (BudgetKind::Depth, 1, |l, n| l.depth = n),
        (BudgetKind::Files, 1, |l, n| l.files = n),
        (BudgetKind::Directives, 2, |l, n| l.directives = n),
        (BudgetKind::Aliases, 2, |l, n| l.aliases = n),
        (BudgetKind::Tokens, 5, |l, n| l.tokens = n),
        (BudgetKind::RetainedBytes, stats.retained, |l, n| {
            l.retained_bytes = n;
        }),
        (BudgetKind::ResolveWork, stats.work, |l, n| {
            l.resolve_work = n;
        }),
    ];
    for (kind, exact, set) in cases {
        let mut admitted = limits;
        set(&mut admitted, exact);
        assert_eq!(f.load(admitted).unwrap().0.len(), 2, "{kind:?} exact");
        set(&mut admitted, exact - 1);
        refusal(f.load(admitted), kind, &f.config());
        assert_eq!(fs::read_to_string(f.config()).unwrap(), body);
    }
    // The final user value, rather than a path or a keyword, is the longest admitted string.
    f.write(
        "config",
        &format!("Host a\nUser {}\n", "u".repeat(limits.string_bytes + 1)),
    );
    refusal(f.load(limits), BudgetKind::StringBytes, &f.config());
    let exact_file = format!(
        "Host a\n#{}",
        "x".repeat(limits.file_bytes - "Host a\n#".len())
    );
    f.write("config", &exact_file);
    assert_eq!(f.load(limits).unwrap().0.len(), 1);
    f.write("config", &(exact_file + "x"));
    refusal(f.load(limits), BudgetKind::FileBytes, &f.config());
}

#[test]
fn includes_bound_depth_file_attempts_and_all_glob_candidates_and_reject_cycles() {
    let f = Fixture::new();
    for dir in ["team-a", "team-b"] {
        fs::create_dir(f.0.join(".ssh").join(dir)).unwrap();
    }
    f.write("config", "Include team-*/*.conf\n");
    f.write("team-a/a.conf", "Host alpha\nUser first\n");
    f.write("team-b/b.conf", "Host beta\nUser second\n");
    f.write("team-b/not-a-match", "ignored");
    let (hosts, stats) = f.load(Limits::default()).unwrap();
    assert_eq!(
        hosts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert!(stats.candidates >= 5);
    for (kind, exact, set) in [
        (
            BudgetKind::GlobCandidates,
            stats.candidates,
            (|l: &mut Limits, n| l.glob_candidates = n) as fn(&mut Limits, usize),
        ),
        (
            BudgetKind::Files,
            stats.files,
            (|l: &mut Limits, n| l.files = n) as fn(&mut Limits, usize),
        ),
        (
            BudgetKind::TotalBytes,
            stats.bytes,
            (|l: &mut Limits, n| l.total_bytes = n) as fn(&mut Limits, usize),
        ),
        (
            BudgetKind::Depth,
            2,
            (|l: &mut Limits, n| l.depth = n) as fn(&mut Limits, usize),
        ),
    ] {
        let mut limits = Limits::default();
        set(&mut limits, exact);
        assert_eq!(f.load(limits).unwrap().0.len(), 2);
        set(&mut limits, exact - 1);
        refusal(f.load(limits), kind, &f.config());
    }
    f.write("config", "Include chain-0\n");
    for n in 0..16 {
        f.write(&format!("chain-{n}"), &format!("Include chain-{}\n", n + 1));
    }
    f.write("chain-16", "Host too-deep\n");
    refusal(f.load(Limits::default()), BudgetKind::Depth, &f.config());
    f.write("config", "Host before\nInclude loop\n");
    f.write("loop", "Include config\n");
    let error = f.load(Limits::default()).unwrap_err();
    assert_eq!(error.kind(), ConfigFailureKind::ConfigInvalid);
    assert!(!error.diagnostic().contains(&f.0.display().to_string()));
    f.write("loop", "Host after\n");
    assert_eq!(f.load(Limits::default()).unwrap().0.len(), 2);
}

#[test]
#[cfg(target_os = "linux")]
fn descriptor_admission_rejects_special_files_growth_truncation_and_substitution() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let body = "Host original\n";
    for change in ["grow", "truncate", "substitute"] {
        f.write("config", body);
        let mut budget = Budget::new(Limits::default(), &|| true);
        let result = io::read_probe(&f.config(), &mut budget, |file| {
            assert!(
                rustix::fs::fcntl_getfl(file)
                    .unwrap()
                    .contains(rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK)
            );
            match change {
                "grow" => f.write("config", &(body.to_owned() + "x")),
                "truncate" => f.write("config", "H"),
                _ => {
                    fs::rename(f.config(), f.0.join("original")).unwrap();
                    f.write("config", "foreign sentinel");
                }
            }
        });
        assert!(result.is_err());
        if change == "substitute" {
            assert_eq!(fs::read_to_string(f.config()).unwrap(), "foreign sentinel");
        }
    }
    fs::remove_file(f.config()).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        f.config(),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let peer = rustix::fs::open(
        f.config(),
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .unwrap();
    assert_eq!(
        f.load(Limits::default()).unwrap_err().kind(),
        ConfigFailureKind::ConfigInvalid
    );
    drop(peer);
    fs::remove_file(f.config()).unwrap();
    assert!(load_user_config(Path::new("/dev/null"), &f.0).is_err());
    assert!(
        load_user_config(
            &PathBuf::from(format!("/proc/{}/comm", std::process::id())),
            &f.0
        )
        .is_err()
    );
    f.write("regular", body);
    symlink(f.0.join(".ssh/regular"), f.config()).unwrap();
    assert!(f.load(Limits::default()).is_err());
    assert_eq!(fs::read_to_string(f.0.join(".ssh/regular")).unwrap(), body);
    fs::remove_file(f.config()).unwrap();
    f.write("config", body);
    assert_eq!(f.load(Limits::default()).unwrap().0[0].alias, "original");
}

#[test]
#[cfg(target_os = "linux")]
fn held_parent_and_all_include_paths_refuse_substitution_and_symlink_ancestors() {
    use std::{cell::Cell, os::unix::fs::symlink};
    let f = Fixture::new();
    f.write("config", "Host original\n");
    let original = fs::File::open(f.config()).unwrap();
    let expected = enzui_core::secure_dir::open_file_identity(&original).unwrap();
    let seen = Cell::new(false);
    let mut budget = Budget::new(Limits::default(), &|| true);
    let result = io::read_parent_probe(
        &f.config(),
        &mut budget,
        || {
            fs::rename(f.0.join(".ssh"), f.0.join("held")).unwrap();
            fs::create_dir(f.0.join(".ssh")).unwrap();
            f.write("config", "Host foreign\n");
        },
        |file| {
            assert_eq!(
                enzui_core::secure_dir::open_file_identity(file).unwrap(),
                expected
            );
            seen.set(true);
        },
    );
    assert!(
        seen.get(),
        "data open did not stay relative to the admitted parent"
    );
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(f.config()).unwrap(), "Host foreign\n");
    symlink(f.0.join("held"), f.0.join(".ssh/link")).unwrap();
    for include in ["link/config", "li*/config"] {
        f.write("config", &format!("Host before\nInclude {include}\n"));
        assert_eq!(
            f.load(Limits::default()).unwrap_err().kind(),
            ConfigFailureKind::ConfigInvalid
        );
    }
    fs::remove_file(f.0.join(".ssh/link")).unwrap();
    f.write("config", "Include ../held/config\n");
    assert_eq!(f.load(Limits::default()).unwrap().0[0].alias, "original");
    fs::rename(f.0.join(".ssh"), f.0.join("directory-backup")).unwrap();
    fs::write(f.0.join(".ssh"), "not a directory").unwrap();
    assert_eq!(
        f.load(Limits::default()).unwrap_err().kind(),
        ConfigFailureKind::NotDirectory
    );
    fs::remove_file(f.0.join(".ssh")).unwrap();
    fs::rename(f.0.join("directory-backup"), f.0.join(".ssh")).unwrap();
    assert_eq!(f.load(Limits::default()).unwrap().0[0].alias, "original");
}

#[test]
fn glob_case_and_host_match_include_order_do_not_reorder_first_values() {
    let f = Fixture::new();
    f.write(
        "a.conf",
        "User included\nMatch exec ignored\nHostName forbidden\n",
    );
    f.write("B.CONF", "Host uppercase-file-must-not-match\n");
    f.write(".hidden.conf", "Host hidden-file-must-not-match\n");
    f.write("z.conf", "Host NEXT\nUser next-user\n");
    f.write("config", "Host alpha\nUser = first\nInclude *.conf\nIdentityFile ~/.ssh/key=a\nHost next alpha\nUser later\n");
    let hosts = f.load(Limits::default()).unwrap().0;
    assert_eq!(
        hosts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(),
        ["alpha", "NEXT"]
    );
    assert_eq!((&*hosts[0].hostname, &*hosts[0].user), ("alpha", "first"));
    assert_eq!(hosts[1].user, "next-user");
    assert_eq!(hosts[1].identity_file.as_deref(), Some("~/.ssh/key=a"));
    let names: Vec<_> = (0..600).map(|n| format!("h{n}")).collect();
    f.write(
        "config",
        &format!("Host {}\nUser common\n", names.join(" ")),
    );
    let (hosts, stats) = f.load(Limits::default()).unwrap();
    assert_eq!(hosts.len(), 600);
    assert!(hosts.iter().all(|h| h.user == "common"));
    assert!(stats.work <= 600 * 256 + 256);
}

#[test]
fn allocation_cancellation_and_deadline_admission_precede_growth_or_reader_hooks() {
    use std::cell::Cell;
    let f = Fixture::new();
    f.write("config", "Host a\n");
    let limits = Limits {
        retained_bytes: 0,
        ..Default::default()
    };
    let mut budget = Budget::new(limits, &|| true);
    let mut values = Vec::<u64>::new();
    assert_eq!(
        budget.push(&mut values, 1).unwrap_err().kind(),
        ConfigFailureKind::Budget
    );
    assert_eq!((values.len(), values.capacity()), (0, 0));
    for mode in ["retained", "cancelled", "deadline"] {
        let live = || mode != "cancelled";
        let mut budget = Budget::new(
            if mode == "retained" {
                limits
            } else {
                Limits::default()
            },
            &live,
        );
        if mode == "deadline" {
            budget.deadline = Instant::now();
        }
        let entered = Cell::new(false);
        let result = io::read_probe(&f.config(), &mut budget, |_| entered.set(true));
        assert!(result.is_err(), "{mode}");
        assert!(!entered.get(), "{mode} reached admitted data");
    }
    assert_eq!(f.load(Limits::default()).unwrap().0[0].alias, "a");
}
