// known_hosts is a trust database, not a source of saved connection aliases.
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[path = "ssh_config_io.rs"]
mod io;
#[path = "ssh_config_resolve.rs"]
mod resolve;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenSshHost {
    pub config_fingerprint: u64,
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub identity_file: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigFailureKind {
    Permission,
    Missing,
    NotDirectory,
    ConfigInvalid,
    Budget,
    Cancelled,
    Busy,
    Other,
}
impl ConfigFailureKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Missing => "missing",
            Self::NotDirectory => "not-directory",
            Self::ConfigInvalid => "config-invalid",
            Self::Budget => "budget",
            Self::Cancelled => "cancelled",
            Self::Busy => "busy",
            Self::Other => "other",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BudgetKind {
    FileBytes,
    TotalBytes,
    Depth,
    Files,
    GlobCandidates,
    Directives,
    Aliases,
    Tokens,
    StringBytes,
    RetainedBytes,
    ResolveWork,
    Deadline,
}
// I/O and content failures retain source diagnostics; budget refusals contain
// no source path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenSshConfigError {
    kind: ConfigFailureKind,
    diagnostic: String,
}
impl OpenSshConfigError {
    fn io(path: &Path, error: std::io::Error) -> Self {
        let kind = match error.kind() {
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
                ConfigFailureKind::Permission
            }
            std::io::ErrorKind::NotFound => ConfigFailureKind::Missing,
            std::io::ErrorKind::NotADirectory => ConfigFailureKind::NotDirectory,
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::InvalidInput => {
                ConfigFailureKind::ConfigInvalid
            }
            _ => ConfigFailureKind::Other,
        };
        Self {
            kind,
            diagnostic: format!("OpenSSH config {}: {error}", path.display()),
        }
    }
    fn invalid(reason: &'static str) -> Self {
        Self {
            kind: ConfigFailureKind::ConfigInvalid,
            diagnostic: reason.into(),
        }
    }
    /// Invalid content of one admitted source, named like an I/O failure so a
    /// log can tell which of several included files is bad.
    fn invalid_at(path: &Path, reason: &'static str) -> Self {
        Self {
            kind: ConfigFailureKind::ConfigInvalid,
            diagnostic: format!("OpenSSH config {}: {reason}", path.display()),
        }
    }
    pub(crate) fn unavailable() -> Self {
        Self {
            kind: ConfigFailureKind::Other,
            diagnostic: "OpenSSH discovery worker is unavailable.".into(),
        }
    }
    pub(crate) fn busy() -> Self {
        Self {
            kind: ConfigFailureKind::Busy,
            diagnostic: "OpenSSH discovery owner is still busy.".into(),
        }
    }
    fn exceeded(budget: BudgetKind) -> Self {
        Self {
            kind: ConfigFailureKind::Budget,
            diagnostic: format!("OpenSSH discovery budget exceeded: {budget:?}."),
        }
    }
    fn cancelled() -> Self {
        Self {
            kind: ConfigFailureKind::Cancelled,
            diagnostic: "OpenSSH discovery superseded.".into(),
        }
    }
    pub(crate) const fn kind(&self) -> ConfigFailureKind {
        self.kind
    }
    pub(crate) const fn user_summary(&self) -> &'static str {
        match self.kind {
            ConfigFailureKind::Permission => {
                "OpenSSH config could not be read because permission was denied."
            }
            ConfigFailureKind::Missing => "No OpenSSH config was found.",
            ConfigFailureKind::ConfigInvalid | ConfigFailureKind::NotDirectory => {
                "OpenSSH config is invalid."
            }
            ConfigFailureKind::Budget => {
                "OpenSSH discovery is unavailable: configuration limits were exceeded."
            }
            ConfigFailureKind::Cancelled => "OpenSSH discovery was superseded.",
            ConfigFailureKind::Busy => {
                "OpenSSH discovery is unavailable: the previous reader is still busy. Reload to retry."
            }
            ConfigFailureKind::Other => "OpenSSH config could not be loaded.",
        }
    }
    // Untrusted path bytes belong only in escaped diagnostics, never directly in UI text.
    pub(crate) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}
impl std::fmt::Display for OpenSshConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.diagnostic)
    }
}
impl std::error::Error for OpenSshConfigError {}
type Result<T> = std::result::Result<T, OpenSshConfigError>;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub file_bytes: usize,
    pub total_bytes: usize,
    pub depth: usize,
    pub files: usize,
    pub glob_candidates: usize,
    pub directives: usize,
    pub aliases: usize,
    pub tokens: usize,
    pub string_bytes: usize,
    pub retained_bytes: usize,
    pub resolve_work: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 256 * 1024,
            total_bytes: 2 * 1024 * 1024,
            depth: 16,
            files: 128,
            glob_candidates: 4096,
            directives: 16_384,
            aliases: 1024,
            tokens: 32_768,
            string_bytes: 4096,
            retained_bytes: 4 * 1024 * 1024,
            resolve_work: 16 * 1024 * 1024,
        }
    }
}
#[derive(Default, Debug)]
pub(crate) struct Stats {
    pub bytes: usize,
    pub files: usize,
    pub candidates: usize,
    pub directives: usize,
    pub aliases: usize,
    pub tokens: usize,
    pub retained: usize,
    pub work: usize,
}
struct Budget<'a> {
    limits: Limits,
    stats: Stats,
    live: &'a dyn Fn() -> bool,
    deadline: Instant,
    work_limit: usize,
}
impl<'a> Budget<'a> {
    fn new(limits: Limits, live: &'a dyn Fn() -> bool) -> Self {
        let now = Instant::now();
        Self {
            work_limit: limits.resolve_work,
            limits,
            stats: Stats::default(),
            live,
            deadline: now.checked_add(Duration::from_secs(5)).unwrap_or(now),
        }
    }
    fn check(&self) -> Result<()> {
        if !(self.live)() {
            return Err(OpenSshConfigError::cancelled());
        }
        if Instant::now() >= self.deadline {
            return Err(OpenSshConfigError::exceeded(BudgetKind::Deadline));
        }
        Ok(())
    }
    fn charge(&mut self, kind: BudgetKind, amount: usize) -> Result<()> {
        self.check()?;
        let (counter, limit) = match kind {
            BudgetKind::TotalBytes => (&mut self.stats.bytes, self.limits.total_bytes),
            BudgetKind::Files => (&mut self.stats.files, self.limits.files),
            BudgetKind::GlobCandidates => (&mut self.stats.candidates, self.limits.glob_candidates),
            BudgetKind::Directives => (&mut self.stats.directives, self.limits.directives),
            BudgetKind::Aliases => (&mut self.stats.aliases, self.limits.aliases),
            BudgetKind::Tokens => (&mut self.stats.tokens, self.limits.tokens),
            BudgetKind::RetainedBytes => (&mut self.stats.retained, self.limits.retained_bytes),
            BudgetKind::ResolveWork => (&mut self.stats.work, self.work_limit),
            _ => return Err(OpenSshConfigError::exceeded(kind)),
        };
        *counter = counter
            .checked_add(amount)
            .filter(|n| *n <= limit)
            .ok_or_else(|| OpenSshConfigError::exceeded(kind))?;
        Ok(())
    }
    fn storage<T>(&mut self, count: usize) -> Result<()> {
        self.charge(
            BudgetKind::RetainedBytes,
            count
                .checked_mul(std::mem::size_of::<T>())
                .ok_or_else(|| OpenSshConfigError::exceeded(BudgetKind::RetainedBytes))?,
        )
    }
    fn push<T>(&mut self, values: &mut Vec<T>, value: T) -> Result<()> {
        if values.len() == values.capacity() {
            let extra = values.capacity().max(4);
            self.storage::<T>(extra)?;
            values
                .try_reserve_exact(extra)
                .map_err(|_| OpenSshConfigError::unavailable())?;
        }
        values.push(value);
        Ok(())
    }
    fn text(&self, text: &str) -> Result<()> {
        if text.len() > self.limits.string_bytes {
            return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
        }
        self.check()
    }
    fn owned(&mut self, text: &str) -> Result<String> {
        self.text(text)?;
        self.charge(BudgetKind::RetainedBytes, text.len())?;
        Ok(text.to_owned())
    }
    fn path(&mut self, path: &Path) -> Result<PathBuf> {
        let bytes = path.as_os_str().as_encoded_bytes().len();
        if bytes > self.limits.string_bytes {
            return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
        }
        self.charge(BudgetKind::RetainedBytes, bytes)?;
        Ok(path.to_owned())
    }
    fn join(&mut self, parent: &Path, part: &std::ffi::OsStr) -> Result<PathBuf> {
        let bytes = parent
            .as_os_str()
            .as_encoded_bytes()
            .len()
            .checked_add(part.as_encoded_bytes().len())
            .and_then(|n| n.checked_add(1));
        if bytes.is_none_or(|n| n > self.limits.string_bytes) {
            return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
        }
        self.charge(BudgetKind::RetainedBytes, bytes.unwrap_or(0))?;
        let mut path = PathBuf::with_capacity(bytes.unwrap_or(0));
        path.push(parent);
        path.push(part);
        Ok(path)
    }
}

#[derive(Default)]
struct Values {
    text: [Option<String>; 4],
}
struct Block {
    patterns: Vec<String>,
    negatives: Vec<String>,
    values: Values,
}
struct Parser {
    blocks: Vec<Block>,
    current: Option<Block>,
    names: Vec<String>,
    aliases: BTreeMap<String, usize>,
    active: Vec<(u64, u64)>,
    fingerprint: std::collections::hash_map::DefaultHasher,
}
impl Parser {
    fn new(budget: &mut Budget<'_>) -> Result<Self> {
        let mut patterns = Vec::new();
        let global = budget.owned("*")?;
        budget.push(&mut patterns, global)?;
        Ok(Self {
            blocks: vec![],
            current: Some(Block {
                patterns,
                negatives: Vec::new(),
                values: Values::default(),
            }),
            names: vec![],
            aliases: BTreeMap::new(),
            active: Vec::new(),
            fingerprint: Default::default(),
        })
    }
    fn finish_block(&mut self, budget: &mut Budget<'_>) -> Result<()> {
        if let Some(block) = self.current.take()
            && block.values.text.iter().any(Option::is_some)
        {
            budget.push(&mut self.blocks, block)?;
        }
        Ok(())
    }
    fn file(
        &mut self,
        path: &Path,
        home: &Path,
        include_root: &Path,
        depth: usize,
        missing_ok: bool,
        budget: &mut Budget<'_>,
    ) -> Result<()> {
        if depth > budget.limits.depth {
            return Err(OpenSshConfigError::exceeded(BudgetKind::Depth));
        }
        budget.charge(BudgetKind::Files, 1)?;
        let input = match io::read(path, &self.active, budget) {
            Ok(input) => input,
            Err(error) if missing_ok && error.kind() == ConfigFailureKind::Missing => return Ok(()),
            Err(error) => return Err(error),
        };
        budget.push(&mut self.active, input.identity)?;
        path.hash(&mut self.fingerprint);
        input.body.hash(&mut self.fingerprint);
        for raw in input.body.lines() {
            budget.check()?;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            budget.charge(BudgetKind::Directives, 1)?;
            let (key, value) = split_directive(line);
            budget.text(key)?;
            budget.text(value)?;
            budget.charge(BudgetKind::Tokens, 1)?;
            for token in value.split_whitespace() {
                budget.text(token)?;
                budget.charge(BudgetKind::Tokens, 1)?;
            }
            if key.eq_ignore_ascii_case("include") {
                for token in value.split_whitespace() {
                    for included in io::expand(token, home, include_root, budget)? {
                        self.file(&included, home, include_root, depth + 1, true, budget)?;
                    }
                }
            } else if key.eq_ignore_ascii_case("host") {
                self.finish_block(budget)?;
                let mut patterns = Vec::new();
                let mut negatives = Vec::new();
                for token in value.split_whitespace() {
                    let mut pattern = budget.owned(token)?;
                    pattern.make_ascii_lowercase();
                    if pattern.starts_with('!') {
                        budget.push(&mut negatives, pattern)?;
                        continue;
                    }
                    if !resolve::has_meta(&pattern) && !self.aliases.contains_key(&pattern) {
                        budget.charge(BudgetKind::Aliases, 1)?;
                        let name = budget.owned(token)?;
                        let index_key = budget.owned(&pattern)?;
                        // Tree admission includes spare keys and child pointers, not just the inserted pair.
                        budget.storage::<(String, usize)>(16)?;
                        budget.storage::<usize>(16)?;
                        self.aliases.insert(index_key, self.names.len());
                        budget.push(&mut self.names, name)?;
                    }
                    budget.push(&mut patterns, pattern)?;
                }
                self.current = Some(Block {
                    patterns,
                    negatives,
                    values: Values::default(),
                });
            } else if key.eq_ignore_ascii_case("match") {
                // Runtime Match predicates are not executed; their blocks stay inactive until the next Host.
                self.finish_block(budget)?;
            } else if let Some(block) = self.current.as_mut() {
                let field = ["hostname", "user", "port", "identityfile"]
                    .iter()
                    .position(|name| key.eq_ignore_ascii_case(name));
                if let Some(field) = field
                    && block.values.text[field].is_none()
                    && (field != 2 || value.parse::<u16>().is_ok())
                {
                    block.values.text[field] = Some(budget.owned(value)?);
                }
            }
        }
        input.verify(path, budget)?;
        self.active.pop();
        Ok(())
    }
}
fn split_directive(line: &str) -> (&str, &str) {
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let value = line[end..].trim_start();
    (
        &line[..end],
        value.strip_prefix('=').unwrap_or(value).trim(),
    )
}

#[cfg(test)]
pub(crate) fn load_user_config(config: &Path, home: &Path) -> Result<Vec<OpenSshHost>> {
    load_with(config, home, Limits::default(), &|| true).map(|(hosts, _)| hosts)
}
#[cfg(test)]
pub(crate) fn load_with(
    config: &Path,
    home: &Path,
    limits: Limits,
    live: &dyn Fn() -> bool,
) -> Result<(Vec<OpenSshHost>, Stats)> {
    let mut budget = Budget::new(limits, live);
    let config = budget.path(config)?;
    let home = budget.path(home)?;
    load_admitted(config, home, budget)
}
fn load_admitted(
    config: PathBuf,
    home: PathBuf,
    mut budget: Budget<'_>,
) -> Result<(Vec<OpenSshHost>, Stats)> {
    let include_root = budget.join(&home, std::ffi::OsStr::new(".ssh"))?;
    let mut parser = Parser::new(&mut budget)?;
    parser.file(&config, &home, &include_root, 1, true, &mut budget)?;
    parser.finish_block(&mut budget)?;
    // Bound pathological wildcard fan-out by input work, not aliases times directives.
    let linear = budget
        .stats
        .bytes
        .saturating_add(budget.stats.tokens)
        .saturating_add(budget.stats.aliases)
        .saturating_add(budget.stats.directives)
        .saturating_add(1)
        .saturating_mul(64);
    budget.stats.work = 0;
    budget.work_limit = budget.limits.resolve_work.min(linear);
    let hosts = resolve::hosts(parser, &mut budget)?;
    Ok((hosts, budget.stats))
}
#[cfg(not(test))]
pub(crate) fn load_default(live: &dyn Fn() -> bool) -> Result<Vec<OpenSshHost>> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(vec![]);
    };
    let mut budget = Budget::new(Limits::default(), live);
    let bytes = home.as_encoded_bytes().len();
    if bytes > budget.limits.string_bytes {
        return Err(OpenSshConfigError::exceeded(BudgetKind::StringBytes));
    }
    budget.charge(BudgetKind::RetainedBytes, bytes)?;
    let home = PathBuf::from(home);
    let config = budget.join(&home, std::ffi::OsStr::new(".ssh/config"))?;
    load_admitted(config, home, budget).map(|(hosts, _)| hosts)
}

#[cfg(test)]
#[path = "ssh_config_budget_tests.rs"]
mod budget_tests;
#[cfg(test)]
#[path = "ssh_config_tests.rs"]
mod tests;
