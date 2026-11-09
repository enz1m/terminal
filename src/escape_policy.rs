//! Bounds and admission rules for untrusted terminal control sequences.
//!
//! Everything here runs on bytes the child process chose, so each entry point
//! answers one question: how many bytes may be retained, and which payloads may
//! become authority (a title, a clickable link, a working directory, PTY
//! input). Refusal is always the quiet outcome — the caller keeps its previous
//! value — except for [`StringSequenceGuard`], whose overflow the pane reports
//! on screen.

use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes one unterminated OSC/DCS/APC/SOS/PM payload may accumulate before the
/// parser is reset and the rest of that sequence discarded.
///
/// The underlying `vtparse` string buffers grow without limit, so a child that
/// opens `ESC ]` and never terminates it would otherwise retain every byte it
/// ever writes. Titles and links are three orders of magnitude smaller than
/// this; sixel and iTerm image payloads are larger, and this app renders
/// neither.
pub(crate) const MAX_STRING_SEQUENCE_BYTES: usize = 64 * 1024;

/// Bytes accepted for an OSC 0/1/2 window title.
pub(crate) const MAX_TITLE_BYTES: usize = 512;

/// Bytes accepted for an OSC 8 hyperlink URI.
pub(crate) const MAX_LINK_BYTES: usize = 2048;

/// Hyperlink ranges retained per projected row.
///
/// One range per cell is reachable by alternating URIs, and every retained row
/// owns its own copies, so the per-row link cost is capped independently of the
/// scrollback bound.
pub(crate) const MAX_LINE_HYPERLINKS: usize = 32;

/// Bytes retained for one grapheme cluster in one cell.
///
/// A combining run is a single cluster of unbounded length; without this cap
/// one cell can hold an entire PTY burst and the cross-chunk cluster rejoin
/// becomes quadratic.
#[cfg(test)]
pub(crate) const MAX_GRAPHEME_BYTES: usize = 64;

/// Bytes of clipboard or history text accepted for one paste.
///
/// One paste is one chunk on the session's input queue, written whole before
/// anything else, so this is also the largest chunk
/// [`crate::pty::MAX_INPUT_CHUNK_BYTES`] is derived from.
pub(crate) const MAX_PASTE_BYTES: usize = 64 * 1024;

/// Which process-wide authority an [`AuthorityAllocator`] mints.
///
/// Each kind reaches its boundary independently, so exhausting one leaves the
/// other minting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityKind {
    /// PTY input generations, minted once per session by [`crate::pty`].
    Input,
    /// Screen link generations, minted by [`crate::emulator`] for each
    /// screen and again on each authority reset.
    Link,
}

/// Mints authority generations that never alias.
///
/// A generation identifies exactly one owner of retained state — a queued input
/// chunk, a projected hyperlink range — so the counter must stay injective.
/// This one refuses rather than wraps: `0` is never minted, because it is the
/// reserved "no authority" value every consumer treats as refusal, and
/// `u64::MAX` is parked rather than handed out, which latches refusal for the
/// rest of the process.
pub(crate) struct AuthorityAllocator {
    /// Selects this allocator's test seam.
    #[cfg(test)]
    kind: AuthorityKind,
    /// The next generation to mint. Never decreases, and stops at `u64::MAX`.
    next: AtomicU64,
}

impl AuthorityAllocator {
    pub(crate) const fn new(_kind: AuthorityKind) -> Self {
        Self {
            #[cfg(test)]
            kind: _kind,
            next: AtomicU64::new(1),
        }
    }

    /// Mint the next generation, or `None` once this allocator is exhausted.
    ///
    /// Callers must treat `None` as terminal: nothing resets an exhausted
    /// allocator, so a refusal cannot be retried into a success.
    pub(crate) fn allocate(&self) -> Option<NonZeroU64> {
        #[cfg(test)]
        if let Some(minted) = seam_allocation(self.kind) {
            return minted;
        }
        self.take_next()
    }

    /// The allocation itself, shared with the test seam's substitute counters.
    fn take_next(&self) -> Option<NonZeroU64> {
        // `u64::MAX` has no successor, so the update fails there and on every
        // later call; a wrapping increment would alias `0` onto a live owner.
        self.next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .ok()
            .and_then(NonZeroU64::new)
    }

    /// A substitute allocator positioned at `start`, used only by the seam.
    #[cfg(test)]
    const fn at(start: u64, kind: AuthorityKind) -> Self {
        Self {
            kind,
            next: AtomicU64::new(start),
        }
    }
}

/// The last generation an allocator can mint: one below the parked value.
#[cfg(test)]
pub(crate) const LAST_AUTHORITY_GENERATION: u64 = u64::MAX - 1;

#[cfg(test)]
thread_local! {
    // Substitutes indexed by `AuthorityKind`. The process-wide allocators are
    // shared by every test in this binary and spending one is irreversible, so
    // a control drives a thread-local counter through the same `take_next`.
    static AUTHORITY_SEAM: std::cell::RefCell<[Option<AuthorityAllocator>; 2]> =
        const { std::cell::RefCell::new([None, None]) };
}

/// The seam's answer for `kind`, or `None` when no seam is engaged here.
#[cfg(test)]
fn seam_allocation(kind: AuthorityKind) -> Option<Option<NonZeroU64>> {
    AUTHORITY_SEAM.with(|seam| {
        seam.borrow()[kind as usize]
            .as_ref()
            .map(AuthorityAllocator::take_next)
    })
}

/// Leaves one authority generation of one kind for the current thread.
///
/// While this is alive, the next allocation of `kind` on this thread mints
/// [`LAST_AUTHORITY_GENERATION`] and every allocation after it is refused.
/// Dropping it restores the process-wide allocator, which the guard never
/// touched.
#[cfg(test)]
pub(crate) struct AuthoritySeam(AuthorityKind);

#[cfg(test)]
impl AuthoritySeam {
    pub(crate) fn nearly_exhausted(kind: AuthorityKind) -> Self {
        AUTHORITY_SEAM.with(|seam| {
            seam.borrow_mut()[kind as usize] =
                Some(AuthorityAllocator::at(LAST_AUTHORITY_GENERATION, kind));
        });
        Self(kind)
    }
}

#[cfg(test)]
impl Drop for AuthoritySeam {
    fn drop(&mut self) {
        AUTHORITY_SEAM.with(|seam| seam.borrow_mut()[self.0 as usize] = None);
    }
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// The bracketed-paste guards, which a payload must never contain.
const PASTE_GUARDS: [&str; 2] = ["\u{1b}[200~", "\u{1b}[201~"];

/// Text that may be shown as terminal-supplied metadata: no controls and no
/// bidirectional overrides or isolates.
fn is_display_safe(text: &str) -> bool {
    crate::pinned_output::plain_text(text)
}

/// An OSC 0/1/2 title accepted as bounded display metadata, or `None` when the
/// payload is empty, oversized, or not display-safe.
pub(crate) fn accepted_title(raw: &str) -> Option<String> {
    let title = raw.trim();
    (!title.is_empty() && title.len() <= MAX_TITLE_BYTES && is_display_safe(title))
        .then(|| title.to_owned())
}

/// An OSC 7 working-directory report accepted as the pane's reported location.
///
/// Only a syntactically valid local `file://` URI qualifies: display-safe,
/// bounded before and after percent-decoding, an absolute path, and no query
/// or fragment. Whether the accepted URI may also become a filesystem base is
/// a separate decision, made by [`resolvable_local_base`] against the label the
/// pane ends up showing, because an authority naming another host is a location
/// this process cannot resolve.
///
/// Anything else is refused outright and the pane keeps its previous location.
pub(crate) fn accepted_cwd_uri(raw: &str) -> Option<String> {
    const SCHEME: &str = "file://";
    let cwd = raw.trim();
    if cwd.len() > crate::shell_history::cwd::MAX_PATH_BYTES
        || !is_display_safe(cwd)
        || cwd.contains(['?', '#'])
        || !cwd.is_char_boundary(SCHEME.len())
        || !cwd[..SCHEME.len()].eq_ignore_ascii_case(SCHEME)
    {
        return None;
    }
    let (_authority, path) = cwd[SCHEME.len()..].split_once('/')?;
    let decoded = percent_decoded_path(path)?;
    (!decoded.is_empty() && decoded.len() < crate::shell_history::cwd::MAX_PATH_BYTES)
        .then(|| cwd.to_owned())
}

/// Decode one percent-encoded OSC 7 path body, refusing NUL and any control
/// byte. The leading `/` consumed by the authority split is restored, so the
/// result is always absolute.
fn percent_decoded_path(path: &str) -> Option<Vec<u8>> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(b'/');
    let mut index = 0;
    while index < bytes.len() {
        let byte = if bytes[index] == b'%' {
            let pair = bytes.get(index + 1..index + 3)?;
            let decoded = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
            index += 3;
            decoded
        } else {
            index += 1;
            bytes[index - 1]
        };
        if byte == 0 || byte.is_ascii_control() {
            return None;
        }
        out.push(byte);
    }
    Some(out)
}

/// The directory a clicked relative path may resolve against, or `None` when
/// the pane's reported location is not a local absolute path.
///
/// A host-qualified label (`host:/srv/app`), a relative fragment, an oversized
/// path and anything carrying a control character all refuse, so a reported
/// location that this process cannot resolve never becomes a filesystem base.
pub(crate) fn resolvable_local_base(label: &str) -> Option<std::path::PathBuf> {
    (label.starts_with('/')
        && label.len() <= crate::shell_history::cwd::MAX_PATH_BYTES
        && !label.chars().any(char::is_control))
    .then(|| std::path::PathBuf::from(label))
}

/// Whether an OSC 8 URI may be retained as a clickable destination.
///
/// Only bounded, control-free, credential-free `http`/`https` URIs qualify. The
/// label an OSC 8 link paints is chosen independently of its target, so a
/// destination that the visible text cannot corroborate — `file:`, a local
/// path, a custom scheme registered to a program, `user:password@host` — is
/// refused rather than shown as an ordinary link.
pub(crate) fn link_is_openable(uri: &str) -> bool {
    if uri.is_empty() || uri.len() > MAX_LINK_BYTES {
        return false;
    }
    // Non-ASCII and ASCII controls must arrive percent-encoded; accepting them
    // raw would let the URI carry terminal control bytes or invisible
    // homographs.
    if !uri.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return false;
    }
    let Some(rest) = strip_scheme(uri) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty() && !authority.contains('@')
}

fn strip_scheme(uri: &str) -> Option<&str> {
    for scheme in ["https://", "http://"] {
        if uri.is_char_boundary(scheme.len()) && uri[..scheme.len()].eq_ignore_ascii_case(scheme) {
            return Some(&uri[scheme.len()..]);
        }
    }
    None
}

/// The prefix of `cluster` that one cell retains.
///
/// Splits on a character boundary at or below [`MAX_GRAPHEME_BYTES`], so a
/// hostile combining run loses its tail instead of the cell's identity.
#[cfg(test)]
pub(crate) fn bounded_grapheme(cluster: &str) -> &str {
    if cluster.len() <= MAX_GRAPHEME_BYTES {
        return cluster;
    }
    let mut end = MAX_GRAPHEME_BYTES;
    while end > 0 && !cluster.is_char_boundary(end) {
        end -= 1;
    }
    &cluster[..end]
}

/// Why a paste was refused instead of normalised.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PasteRefusal {
    /// The payload exceeds [`MAX_PASTE_BYTES`].
    TooLarge,
}

impl PasteRefusal {
    /// One bounded sentence naming the refusal for the pane's failure banner.
    pub(crate) fn user_summary(self) -> &'static str {
        match self {
            Self::TooLarge => "Paste refused: the clipboard holds more text than one paste allows.",
        }
    }
}

/// The exact text a paste may write to the PTY.
///
/// Line endings collapse to `\n` so no policy can be bypassed by choosing a CR
/// variant, the bracketed-paste guards are removed so a payload cannot close
/// its own envelope and have its tail read as typed input, and every remaining
/// control character and bidirectional override is dropped. Tab and newline
/// survive. The result may be empty; callers must not write an empty paste.
///
/// # Errors
///
/// Returns [`PasteRefusal::TooLarge`] when `text` exceeds [`MAX_PASTE_BYTES`].
pub(crate) fn normalized_paste(text: &str) -> Result<String, PasteRefusal> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(PasteRefusal::TooLarge);
    }
    let mut stripped = text.to_owned();
    for guard in PASTE_GUARDS {
        if stripped.contains(guard) {
            stripped = stripped.replace(guard, "");
        }
    }
    let mut out = String::with_capacity(stripped.len());
    let mut chars = stripped.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(ch),
            _ if ch.is_control() => {}
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            _ => out.push(ch),
        }
    }
    Ok(out)
}

/// One run of a PTY chunk, classified by [`StringSequenceGuard::split`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Segment {
    /// Bytes to hand to the parser.
    Feed(Range<usize>),
    /// A run of bytes dropped because the pending string sequence outgrew its
    /// bound. `started` marks the run that begins an unreported overflow.
    Discard { started: bool },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Phase {
    #[default]
    Ground,
    Escape,
    String,
    StringEscape,
}

/// Tracks how many bytes the parser is holding for an unterminated OSC, DCS,
/// APC, SOS or PM sequence, across chunk boundaries.
///
/// The guard recognises only string-sequence framing, which is enough to bound
/// the parser's growing buffers: CSI parameters and intermediates are already
/// fixed-size upstream. Once a payload passes [`MAX_STRING_SEQUENCE_BYTES`] the
/// guard withholds the remainder of that sequence, including its terminator, so
/// the caller can drop the partially buffered sequence with its parser and
/// resume in the ground state.
#[derive(Debug, Default)]
pub(crate) struct StringSequenceGuard {
    phase: Phase,
    pending: usize,
    discarding: bool,
    announced: bool,
    /// Continuation bytes still expected by the current UTF-8 sequence, and
    /// whether that sequence started with 0xC2 (the lead of encoded C1 controls).
    utf8_remaining: u8,
    after_c2: bool,
}

impl StringSequenceGuard {
    /// Classifies `bytes` into runs to feed and runs to drop, advancing the
    /// guard's cross-chunk state. Ranges tile `bytes` in order.
    pub(crate) fn split(&mut self, bytes: &[u8]) -> Vec<Segment> {
        let mut segments = Vec::new();
        let mut cut = 0usize;
        let mut index = 0usize;
        while index < bytes.len() {
            // Plain ASCII text in the ground state changes nothing: skip the
            // run up to the next ESC or non-ASCII byte without stepping.
            if matches!(self.phase, Phase::Ground) && self.utf8_remaining == 0 && !self.after_c2 {
                index = skip_ground_text(bytes, index);
                if index == bytes.len() {
                    break;
                }
            }
            // A string payload changes nothing but its length until a byte
            // that ends it, escapes, or may be a C1 control: count the run.
            if matches!(self.phase, Phase::String) && self.utf8_remaining == 0 && !self.after_c2 {
                let run = bytes[index..]
                    .iter()
                    .position(|&b| matches!(b, BEL | CAN | SUB | ESC) || b >= 0x80)
                    .unwrap_or(bytes.len() - index);
                let counted = if self.discarding {
                    run
                } else {
                    run.min(MAX_STRING_SEQUENCE_BYTES.saturating_sub(self.pending))
                };
                self.pending = self.pending.saturating_add(counted);
                index += counted;
                if index == bytes.len() {
                    break;
                }
            }
            let byte = &bytes[index];
            let was_discarding = self.discarding;
            self.step(*byte);
            match (was_discarding, self.discarding) {
                (false, true) => {
                    if cut < index {
                        segments.push(Segment::Feed(cut..index));
                    }
                    cut = index;
                }
                (true, false) => {
                    segments.push(self.discard());
                    cut = index + 1;
                }
                _ => {}
            }
            index += 1;
        }
        if self.discarding {
            segments.push(self.discard());
        } else if cut < bytes.len() {
            segments.push(Segment::Feed(cut..bytes.len()));
        }
        segments
    }

    fn discard(&mut self) -> Segment {
        let started = !self.announced;
        self.announced = true;
        Segment::Discard { started }
    }

    /// Classify `byte` as a C1 control the parser acts on: either a stray 8-bit
    /// C1 byte outside any UTF-8 sequence, or the second byte of a `C2 80..9F`
    /// UTF-8-encoded C1. Plain continuation bytes of other characters are not.
    fn c1(&mut self, byte: u8) -> Option<u8> {
        if self.utf8_remaining > 0 && (0x80..=0xbf).contains(&byte) {
            self.utf8_remaining -= 1;
            let encoded =
                self.after_c2 && self.utf8_remaining == 0 && (0x80..=0x9f).contains(&byte);
            if self.utf8_remaining == 0 {
                self.after_c2 = false;
            }
            return encoded.then_some(byte);
        }
        self.after_c2 = false;
        self.utf8_remaining = match byte {
            0xc2..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf4 => 3,
            _ => 0,
        };
        if byte == 0xc2 {
            self.after_c2 = true;
        }
        (0x80..=0x9f).contains(&byte).then_some(byte)
    }

    fn step(&mut self, byte: u8) {
        let c1 = self.c1(byte);
        match self.phase {
            Phase::Ground => {
                if byte == ESC {
                    self.phase = Phase::Escape;
                } else if c1.is_some_and(introduces_c1_string) {
                    // 8-bit and UTF-8-encoded DCS/SOS/OSC/PM/APC open the same
                    // unbounded parser buffers as their 7-bit forms.
                    self.begin_string();
                }
            }
            Phase::Escape => match byte {
                ESC => {}
                _ if introduces_string(byte) => self.begin_string(),
                _ => self.phase = Phase::Ground,
            },
            Phase::String => match byte {
                BEL | CAN | SUB => self.end_string(),
                _ if c1 == Some(ST_C1) => self.end_string(),
                ESC => self.phase = Phase::StringEscape,
                _ => {
                    self.pending = self.pending.saturating_add(1);
                    if self.pending > MAX_STRING_SEQUENCE_BYTES && !self.discarding {
                        self.discarding = true;
                        self.announced = false;
                    }
                }
            },
            Phase::StringEscape => match byte {
                ESC => {}
                b'\\' => self.end_string(),
                _ if introduces_string(byte) => self.begin_string(),
                _ => {
                    self.end_string();
                }
            },
        }
    }

    fn begin_string(&mut self) {
        self.phase = Phase::String;
        self.pending = 0;
        self.discarding = false;
    }

    fn end_string(&mut self) {
        self.phase = Phase::Ground;
        self.pending = 0;
        self.discarding = false;
    }
}

/// The end of the text from `index` that leaves a ground-state guard as it
/// is: ASCII other than ESC, and whole UTF-8 characters whose lead cannot
/// start an encoded C1 (`C2 80..9F`). Anything else, or a character cut by
/// the chunk's end, is left to [`StringSequenceGuard::step`].
fn skip_ground_text(bytes: &[u8], mut index: usize) -> usize {
    while let Some(&byte) = bytes.get(index) {
        if byte < 0x80 {
            if byte == ESC {
                break;
            }
            index += 1;
            continue;
        }
        let continuation = match byte {
            0xc3..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf4 => 3,
            _ => break,
        };
        match bytes.get(index + 1..=index + continuation) {
            Some(rest) if rest.iter().all(|b| (0x80..=0xbf).contains(b)) => {
                index += 1 + continuation;
            }
            _ => break,
        }
    }
    index
}

fn introduces_string(byte: u8) -> bool {
    matches!(byte, b']' | b'P' | b'_' | b'^' | b'X')
}

/// C1 DCS, SOS, OSC, PM and APC.
fn introduces_c1_string(byte: u8) -> bool {
    matches!(byte, 0x90 | 0x98 | 0x9d | 0x9e | 0x9f)
}

/// C1 string terminator.
const ST_C1: u8 = 0x9c;

#[cfg(test)]
mod tests {
    use super::*;

    /// Random PTY output mixing text, multi-byte and C1 bytes, controls and
    /// string sequences longer than the bound.
    pub(crate) fn mixed_stream(seed: u64, len: usize) -> Vec<u8> {
        let mut state = seed | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let pieces: [&[u8]; 16] = [
            b"plain text ",
            "\u{4e16}\u{754c} \u{1f600}".as_bytes(),
            b"\xc2\x9d0;t",
            b"\xc2\x9c",
            b"\x9d",
            b"\x9c",
            b"\x1b]0;",
            b"\x1b]6;",
            b"\x1b_Ga=t;",
            b"\x1bP1$r",
            b"\x07",
            b"\x1b\\",
            b"\x18",
            b"\x1a",
            b"\x1b[31m",
            b"\xe4\xb8",
        ];
        let mut out = Vec::new();
        while out.len() < len {
            let roll = next();
            if roll % 9 == 0 {
                let n = (next() % (MAX_STRING_SEQUENCE_BYTES as u64 + 64)) as usize;
                out.extend(std::iter::repeat_n(b'x', n));
            } else {
                out.extend(pieces[(roll % 16) as usize]);
            }
        }
        out
    }

    /// The bytes fed and the overflows announced, splitting `input` in
    /// chunks of `size`.
    fn guarded(input: &[u8], size: usize) -> (Vec<u8>, usize) {
        let mut guard = StringSequenceGuard::default();
        let (mut fed, mut overflows) = (Vec::new(), 0);
        for chunk in input.chunks(size) {
            for segment in guard.split(chunk) {
                match segment {
                    Segment::Feed(range) => fed.extend(&chunk[range]),
                    Segment::Discard { started } => overflows += usize::from(started),
                }
            }
        }
        (fed, overflows)
    }

    /// The runs the guard skips in bulk classify exactly as stepping each
    /// byte does. Sabotage: count a string run one byte short; the bound then
    /// trips a byte later than stepping trips it.
    #[test]
    fn bulk_runs_classify_as_single_bytes_do() {
        for seed in 1..40 {
            let input = mixed_stream(seed, 3 * MAX_STRING_SEQUENCE_BYTES);
            let stepped = guarded(&input, 1);
            for size in [7, 4096, input.len()] {
                assert_eq!(guarded(&input, size), stepped, "seed {seed} chunk {size}");
            }
        }
    }

    #[test]
    fn eight_bit_and_utf8_encoded_c1_strings_are_bounded() {
        for introducer in [
            &[0x9d_u8][..],
            &[0xc2, 0x9d],
            &[0x90, b'q'],
            &[0xc2, 0x90, b'q'],
        ] {
            let mut guard = StringSequenceGuard::default();
            let mut input = introducer.to_vec();
            input.extend(std::iter::repeat_n(b'A', MAX_STRING_SEQUENCE_BYTES + 10));
            let segments = guard.split(&input);
            assert!(
                segments
                    .iter()
                    .any(|s| matches!(s, Segment::Discard { .. })),
                "{introducer:?}"
            );
        }
        // Ordinary UTF-8 (a continuation byte of 0x9d inside “ or emoji) is text.
        let mut guard = StringSequenceGuard::default();
        let text = "quote \u{201d} \u{1f41d} ".repeat(20_000);
        assert!(
            guard
                .split(text.as_bytes())
                .iter()
                .all(|s| matches!(s, Segment::Feed(_)))
        );
        // C2 9C terminates a UTF-8-encoded OSC; following text is not counted.
        let mut guard = StringSequenceGuard::default();
        let mut input = b"\xc2\x9d0;title\xc2\x9c".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_STRING_SEQUENCE_BYTES + 10));
        assert!(
            guard
                .split(&input)
                .iter()
                .all(|s| matches!(s, Segment::Feed(_)))
        );
    }

    #[test]
    fn only_bounded_credential_free_web_links_are_openable() {
        assert!(link_is_openable("https://example.com/docs?q=1#a"));
        assert!(link_is_openable("HTTP://example.com/"));
        assert!(!link_is_openable("file:///etc/passwd"));
        assert!(!link_is_openable("file://localhost/home/user/x.desktop"));
        assert!(!link_is_openable("https://user:secret@example.com/"));
        assert!(!link_is_openable("https://"));
        assert!(!link_is_openable("javascript:alert(1)"));
        assert!(!link_is_openable("ssh://host/cmd"));
        assert!(!link_is_openable("https://example.com/\u{1b}[31m"));
        assert!(!link_is_openable("https://exa\u{202e}mple.com/"));
        assert!(!link_is_openable(&format!(
            "https://example.com/{}",
            "a".repeat(MAX_LINK_BYTES)
        )));
    }

    #[test]
    fn titles_and_working_directories_refuse_control_and_oversize_payloads() {
        assert_eq!(accepted_title(" shell title "), Some("shell title".into()));
        assert_eq!(accepted_title(""), None);
        assert_eq!(accepted_title("tab\u{7f}title"), None);
        assert_eq!(accepted_title("tab\u{202e}title"), None);
        assert_eq!(accepted_title(&"t".repeat(MAX_TITLE_BYTES + 1)), None);
        assert_eq!(
            accepted_cwd_uri("file://localhost/tmp/x"),
            Some("file://localhost/tmp/x".into())
        );
        assert_eq!(
            accepted_cwd_uri("file://child.example/srv/app"),
            Some("file://child.example/srv/app".into())
        );
        assert_eq!(accepted_cwd_uri("/tmp/plain"), None);
        assert_eq!(accepted_cwd_uri("file://localhost/tmp/x?a=1"), None);
        assert_eq!(accepted_cwd_uri("file://localhost/tmp/%00x"), None);
        assert_eq!(accepted_cwd_uri("file://localhost/tmp/%1bx"), None);
        assert_eq!(accepted_cwd_uri("file://localhost"), None);
        assert_eq!(accepted_cwd_uri("/tmp/\u{9b}31m"), None);

        assert!(resolvable_local_base("/work/app").is_some());
        assert!(resolvable_local_base("host.example:/srv/app").is_none());
        assert!(resolvable_local_base("relative/path").is_none());
    }

    #[test]
    fn a_combining_run_is_truncated_on_a_character_boundary() {
        let cluster = format!("e{}", "\u{301}".repeat(1000));
        let bounded = bounded_grapheme(&cluster);
        assert!(bounded.len() <= MAX_GRAPHEME_BYTES);
        assert!(cluster.starts_with(bounded));
        assert_eq!(bounded_grapheme("\u{1f1fa}\u{1f1f8}"), "\u{1f1fa}\u{1f1f8}");
    }

    #[test]
    fn an_unterminated_sequence_overflows_once_and_resumes_after_its_terminator() {
        let mut guard = StringSequenceGuard::default();
        assert_eq!(
            guard.split(b"text\x1b]0;title\x07more"),
            vec![Segment::Feed(0..18)]
        );

        let mut guard = StringSequenceGuard::default();
        assert_eq!(guard.split(b"\x1b]"), vec![Segment::Feed(0..2)]);
        let payload = vec![b'A'; MAX_STRING_SEQUENCE_BYTES];
        assert_eq!(
            guard.split(&payload),
            vec![Segment::Feed(0..MAX_STRING_SEQUENCE_BYTES)]
        );
        assert_eq!(guard.split(b"BB"), vec![Segment::Discard { started: true }]);
        // The terminator is dropped with the payload, so the resumed feed
        // starts after the BEL.
        assert_eq!(
            guard.split(b"CC\x07tail"),
            vec![Segment::Discard { started: false }, Segment::Feed(3..7)]
        );
    }
}
