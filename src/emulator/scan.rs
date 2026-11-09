//! The few sequences the app acts on that the emulator does not report.
//!
//! `alacritty_terminal` parses the stream itself, but drops OSC 7 (working
//! directory), OSC 133 (shell integration) and OSC 777 (this app's command
//! history), and gives no signal for clearing the scrollback (`CSI 3 J`) or a
//! full reset (`ESC c`). This scanner runs over the same bytes, alongside the
//! emulator, and reports each of those with the byte offset just past its
//! end, so the screen can feed the emulator up to exactly that point first.
//!
//! It tracks only enough of the escape grammar to stay in step: CSI
//! parameters, OSC payloads, and the other string sequences (DCS, SOS, PM,
//! APC), whose contents are skipped so an OSC-looking payload inside them is
//! never mistaken for one. CAN and SUB abort a sequence, as in the emulator.
//! Payloads are bounded; the string-sequence guard has already cut anything
//! longer before the bytes get here.

use crate::escape_policy;

/// A sequence the screen handles itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scanned {
    /// OSC 7: the shell reported its working directory URI.
    Cwd(String),
    /// OSC 133 D: the last command finished with this status.
    CommandStatus(i32),
    /// OSC 777: an rxvt extension, its fields after `777`.
    Rxvt(Vec<String>),
    /// CSI 3 J: erase the scrollback.
    ClearScrollback,
    /// ESC c: full reset.
    FullReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    /// ESC seen inside an OSC: `\` ends it.
    OscEscape,
    /// DCS, SOS, PM or APC contents, skipped.
    Skip,
    /// ESC seen inside a skipped string.
    SkipEscape,
}

#[derive(Debug, Default)]
pub(crate) struct Scanner {
    state: State,
    csi: Vec<u8>,
    osc: Vec<u8>,
    overflow: bool,
}

const MAX_CSI: usize = 64;

impl Scanner {
    /// Scans `bytes`, continuing any sequence the previous call left open,
    /// and reports each recognised sequence with the offset just past it.
    pub(crate) fn scan(&mut self, bytes: &[u8]) -> Vec<(usize, Scanned)> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            match self.state {
                State::Ground => {
                    // Text: jump to the next byte that can start or cancel a sequence.
                    match memchr::memchr3(0x1b, 0x18, 0x1a, &bytes[i..]) {
                        Some(at) => i += at,
                        None => break,
                    }
                }
                State::Skip => {
                    // A DCS/SOS/PM/APC payload is skipped up to the byte that
                    // may end it.
                    match memchr::memchr3(0x1b, 0x18, 0x1a, &bytes[i..]) {
                        Some(at) => i += at,
                        None => break,
                    }
                }
                State::Csi => {
                    // Parameters and intermediates are kept (up to the bound)
                    // in bulk until the final byte or one that ends the CSI.
                    let run = bytes[i..]
                        .iter()
                        .position(|&b| {
                            (0x40..=0x7e).contains(&b) || matches!(b, 0x18 | 0x1a | 0x1b)
                        })
                        .unwrap_or(bytes.len() - i);
                    let room = MAX_CSI.saturating_sub(self.csi.len());
                    self.csi.extend_from_slice(&bytes[i..i + run.min(room)]);
                    i += run;
                    if i == bytes.len() {
                        break;
                    }
                }
                State::Osc => {
                    // An OSC payload is kept (up to the bound) in bulk until
                    // the byte that may end it.
                    let run = bytes[i..]
                        .iter()
                        .position(|&b| matches!(b, 0x07 | 0x18 | 0x1a | 0x1b))
                        .unwrap_or(bytes.len() - i);
                    let room =
                        escape_policy::MAX_STRING_SEQUENCE_BYTES.saturating_sub(self.osc.len());
                    self.osc.extend_from_slice(&bytes[i..i + run.min(room)]);
                    self.overflow |= run > room;
                    i += run;
                    if i == bytes.len() {
                        break;
                    }
                }
                _ => {}
            }
            let b = bytes[i];
            self.step(b, i, &mut out);
            i += 1;
        }
        out
    }

    fn step(&mut self, b: u8, i: usize, out: &mut Vec<(usize, Scanned)>) {
        {
            // CAN and SUB cancel any sequence in progress.
            if b == 0x18 || b == 0x1a {
                self.state = State::Ground;
                return;
            }
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Escape;
                    }
                }
                State::Escape => match b {
                    b'[' => {
                        self.csi.clear();
                        self.state = State::Csi;
                    }
                    b']' => {
                        self.osc.clear();
                        self.overflow = false;
                        self.state = State::Osc;
                    }
                    b'P' | b'X' | b'^' | b'_' => self.state = State::Skip,
                    b'c' => {
                        out.push((i + 1, Scanned::FullReset));
                        self.state = State::Ground;
                    }
                    0x1b => {}
                    // Intermediates keep the escape open (e.g. ESC ( B).
                    0x20..=0x2f => {}
                    _ => self.state = State::Ground,
                },
                State::Csi => match b {
                    0x40..=0x7e => {
                        if b == b'J' && self.csi == b"3" {
                            out.push((i + 1, Scanned::ClearScrollback));
                        }
                        self.state = State::Ground;
                    }
                    0x1b => self.state = State::Escape,
                    _ => {
                        if self.csi.len() < MAX_CSI {
                            self.csi.push(b);
                        }
                    }
                },
                State::Osc => match b {
                    0x07 => {
                        self.finish_osc(i + 1, out);
                        self.state = State::Ground;
                    }
                    0x1b => self.state = State::OscEscape,
                    _ => {
                        if self.osc.len() < escape_policy::MAX_STRING_SEQUENCE_BYTES {
                            self.osc.push(b);
                        } else {
                            self.overflow = true;
                        }
                    }
                },
                State::OscEscape => {
                    if b == b'\\' {
                        self.finish_osc(i + 1, out);
                        self.state = State::Ground;
                    } else {
                        // Any other byte after ESC ends the OSC unterminated
                        // and starts a new escape, as in the emulator.
                        self.state = self.scan_one_after_escape(b, i, out);
                    }
                }
                State::Skip => {
                    if b == 0x1b {
                        self.state = State::SkipEscape;
                    }
                }
                State::SkipEscape => {
                    self.state = if b == b'\\' {
                        State::Ground
                    } else {
                        State::Skip
                    };
                }
            }
        }
    }

    /// The state after an escape's first byte `b` at offset `i`.
    fn scan_one_after_escape(&mut self, b: u8, i: usize, out: &mut Vec<(usize, Scanned)>) -> State {
        match b {
            b'[' => {
                self.csi.clear();
                State::Csi
            }
            b']' => {
                self.osc.clear();
                self.overflow = false;
                State::Osc
            }
            b'P' | b'X' | b'^' | b'_' => State::Skip,
            b'c' => {
                out.push((i + 1, Scanned::FullReset));
                State::Ground
            }
            0x1b | 0x20..=0x2f => State::Escape,
            _ => State::Ground,
        }
    }

    fn finish_osc(&mut self, end: usize, out: &mut Vec<(usize, Scanned)>) {
        if self.overflow {
            return;
        }
        if let Some(scanned) = parse_osc(&self.osc) {
            out.push((end, scanned));
        }
    }

    /// Forgets any sequence in progress (the parser was reset).
    pub(crate) fn reset(&mut self) {
        self.state = State::Ground;
        self.csi.clear();
        self.osc.clear();
        self.overflow = false;
    }
}

fn parse_osc(payload: &[u8]) -> Option<Scanned> {
    let mut fields = payload.split(|&b| b == b';');
    let number = fields.next()?;
    match number {
        b"7" => {
            let rest = payload.get(2..).unwrap_or_default();
            Some(Scanned::Cwd(String::from_utf8_lossy(rest).into_owned()))
        }
        b"133" => {
            if fields.next()? != b"D" {
                return None;
            }
            let status = fields
                .next()
                .and_then(|s| std::str::from_utf8(s).ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            Some(Scanned::CommandStatus(status))
        }
        b"777" => Some(Scanned::Rxvt(
            fields
                .map(|field| String::from_utf8_lossy(field).into_owned())
                .collect(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sequence_split_across_chunks_is_reported_in_the_second() {
        let mut scanner = Scanner::default();
        assert!(scanner.scan(b"\x1b]777;enzui-com").is_empty());
        assert_eq!(
            scanner.scan(b"mand;6c73\x07"),
            vec![(
                10,
                Scanned::Rxvt(vec!["enzui-command".into(), "6c73".into()])
            )]
        );
    }

    #[test]
    fn osc_inside_dcs_and_other_csi_are_ignored() {
        let mut scanner = Scanner::default();
        assert!(scanner.scan(b"\x1bP\x1b]7;x\x07\x1b\\").is_empty());
        assert!(
            scanner
                .scan(b"\x1b[2J\x1b[13J\x1b]133;A\x07\x1b]8;;u\x07")
                .is_empty()
        );
        // CAN aborts an OSC.
        assert!(scanner.scan(b"\x1b]7;x\x18\x07").is_empty());
    }

    /// What `input` reports, scanned in chunks of `size`, at absolute offsets.
    fn scanned(input: &[u8], size: usize) -> Vec<(usize, Scanned)> {
        let mut scanner = Scanner::default();
        let mut out = Vec::new();
        for (index, chunk) in input.chunks(size).enumerate() {
            out.extend(
                scanner
                    .scan(chunk)
                    .into_iter()
                    .map(|(end, found)| (index * size + end, found)),
            );
        }
        out
    }

    /// Payloads skipped and kept in bulk report what stepping each byte
    /// reports. Sabotage: keep one byte fewer of each OSC run; the paths and
    /// commands then lose a character.
    #[test]
    fn bulk_runs_scan_as_single_bytes_do() {
        let pieces: [&[u8]; 14] = [
            b"text ",
            b"\x1b]7;file://host/home/a",
            b"\x1b]133;D;0",
            b"\x1b]777;notify;t;b",
            b"\x07",
            b"\x1b\\",
            b"\x1b]6;",
            b"\x1b_Ga=t;AAAA",
            b"\x1bP1$r",
            b"\x18",
            b"\x1a",
            b"\x1b[3J",
            b"\x1bc",
            b"\x1b[03",
        ];
        for seed in 1..60u64 {
            let mut state = seed;
            let mut input = Vec::new();
            while input.len() < 4 * escape_policy::MAX_STRING_SEQUENCE_BYTES / 3 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                if state % 11 == 0 {
                    let n = (state >> 8) as usize % (escape_policy::MAX_STRING_SEQUENCE_BYTES + 32);
                    input.extend(std::iter::repeat_n(b'y', n));
                } else {
                    input.extend(pieces[(state % 14) as usize]);
                }
            }
            let stepped = scanned(&input, 1);
            for size in [5, 4096, input.len()] {
                assert_eq!(scanned(&input, size), stepped, "seed {seed} chunk {size}");
            }
        }
    }

    /// Sabotage: keep no CSI parameters. Clearing the scrollback is then
    /// never noticed.
    #[test]
    fn clearing_the_scrollback_is_noticed_whole_or_split() {
        let mut scanner = Scanner::default();
        let found = scanner.scan(b"a\x1b[3Jb\x1b[13J\x1b[3J");
        assert_eq!(
            found,
            vec![
                (5, Scanned::ClearScrollback),
                (15, Scanned::ClearScrollback)
            ]
        );
        assert!(scanner.scan(b"\x1b[").is_empty());
        assert_eq!(scanner.scan(b"3J"), vec![(2, Scanned::ClearScrollback)]);
    }
}
