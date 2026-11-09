use super::*;

/// The cached [`Screen::project`] must equal a from-scratch projection of the
/// same state, row for row and cell for cell.
fn assert_equivalent(screen: &mut Screen, sid: &str) {
    let fast = screen.project(sid);
    let reference = screen.project_deep_reference(sid);
    assert_eq!(fast.len(), reference.len(), "row count differs");
    for (i, (f, r)) in fast.iter().zip(reference.iter()).enumerate() {
        assert_eq!(
            **f, *r,
            "row {i} differs between cached and fresh projection"
        );
    }
}

#[path = "tests/deferred_tests.rs"]
mod deferred_tests;

#[path = "tests/osc_tests.rs"]
mod osc_tests;

#[path = "tests/scrollback_tests.rs"]
mod scrollback_tests;

fn screen(rows: usize, cols: usize) -> Screen {
    let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
    screen.resize(rows, cols);
    screen
}

/// A live pane's rows are the same cells the app would write for them:
/// `live_row` equals projection followed by `terminal_view::row_cells`.
///
/// Sabotage: drop the continuation cell or the broken-glyph rule in
/// `live_row` → reddens on the wide and edited rows.
#[test]
fn live_rows_match_the_app_path_cell_for_cell() {
    let streams: &[&[u8]] = &[
        b"plain text\r\n\x1b[31mred\x1b[0m \x1b[1;32mbold green\x1b[0m \x1b[7minverse\x1b[0m",
        b"\x1b[3mitalic\x1b[0m \x1b[4:3mcurly\x1b[0m \x1b[9mstrike\x1b[0m \x1b[2mdim\x1b[0m \x1b[8mhid\x1b[0m",
        "\u{6C34}\u{672C} e\u{301} \u{1F1FA}\u{1F1F8} \u{1F44D}\u{1F3FD} \u{1F468}\u{200D}\u{1F469}".as_bytes(),
        "A\u{754C}BC\x1b[1;3H\x1b[P".as_bytes(),
        "\x1b[48;5;196m  \x1b[0m\x1b[38;2;1;2;3mtrue\x1b[0m\tTab\x1b]8;;https://example.org\x07link\x1b]8;;\x07".as_bytes(),
        "0123456789abcdefghi\u{754C}wrap".as_bytes(),
        "\x1b[44;4m\u{6C34}\x1b[0m \x1b[7m\u{754C}\x1b[0m".as_bytes(),
    ];
    let _scheme = crate::color_scheme::steady_scheme();
    for (i, stream) in streams.iter().enumerate() {
        let mut s = screen(4, 20);
        s.process(stream);
        let (cols, rows) = s.dims();
        let overlays = crate::terminal_view::Overlays {
            focused: true,
            cursor_visible: true,
            selection: None,
            window_start_row: 0,
            matches: &[],
        };
        let fills = crate::terminal_view::Fills::current();
        for r in 0..rows {
            let cells = s.row_cells(r);
            let line = project_cells(cells, "t", false, 0, cols, s.link_generation);
            let app = crate::terminal_view::row_cells(&line, r, &overlays, &fills);
            let live = project::live_row(cells, cols);
            assert_eq!(live, app, "stream {i} row {r}");
        }
    }
}

/// Copying a row from the emulator's written cells keeps exactly what
/// copying every cell keeps, whatever the program did to the row.
///
/// Sabotage: make `copy_row` stop at `occupied()` even when the last cell has
/// a background colour. The coloured-erase rows then lose their fill.
#[test]
fn a_row_copied_from_its_written_cells_keeps_what_a_full_copy_keeps() {
    use alacritty_terminal::index::Line;
    fn cells(row: &RetainedRow) -> Vec<(char, Vec<char>, String, String, u16)> {
        row.views()
            .map(|v| {
                (
                    v.c,
                    v.zerowidth.to_vec(),
                    format!("{:?}", v.fg),
                    format!("{:?}", v.bg),
                    v.flags.bits(),
                )
            })
            .collect()
    }
    let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
    screen.resize(12, 60);
    let steps: [&[u8]; 11] = [
        // A line that scrolls in while a background is set is reset to it.
        b"\x1b[12;1H\x1b[46m\n\n\x1b[0m",
        b"plain text\r\n",
        b"\x1b[41mred fill to the end\x1b[K\x1b[0m\r\n",
        b"written then erased\r\x1b[K\r\n",
        b"\x1b[44m\x1b[2K\x1b[0m\r\n",
        b"far\x1b[58Gxy\r\n",
        b"abcdef\x1b[3G\x1b[2P\x1b[2@\r\n",
        b"\x1b[7m   \x1b[0m\r\n",
        "wide \u{4e16}\u{754c} e\u{301}\r\n".as_bytes(),
        b"\x1b[42mgreen\x1b[0m tail\x1b[1;1H\x1b[43m\x1b[K\x1b[0m",
        b"\x1b[12;1H\r\n\x1b[45m\x1b[J\x1b[0m",
    ];
    for step in steps {
        screen.process(step);
        for line in 0..12 {
            let row = &screen.term.grid()[Line(line)];
            let full = RetainedRow::copy(&row[alacritty_terminal::index::Column(0)..]);
            let fast = RetainedRow::copy_row(row);
            assert_eq!(
                cells(&fast),
                cells(&full),
                "row {line} after {:?}",
                String::from_utf8_lossy(step)
            );
        }
    }
}

/// Runs of ASCII written a row at a time leave the emulator exactly as one
/// input per character does, through wraps, scroll regions, insert mode, the
/// line-drawing charset, wide characters and no-wrap mode.
///
/// Sabotage: in `Term::input_ascii`, skip the wide-character check. Writing
/// over half of a wide character then leaves its other half behind.
#[test]
fn ascii_runs_write_what_single_characters_write() {
    use alacritty_terminal::vte::ansi::{
        CharsetIndex, Handler, Mode, NamedMode, NamedPrivateMode, PrivateMode, StandardCharset,
    };
    let run = |ops: &[(u8, u32)], bulk: bool| {
        let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
        screen.resize(6, 13);
        let term = &mut screen.term;
        for &(op, arg) in ops {
            match op {
                0 => {
                    let text: String = (0..(arg % 40) + 1)
                        .map(|i| (b'!' + ((arg + i) % 90) as u8) as char)
                        .collect();
                    if bulk {
                        term.input_ascii(&text);
                    } else {
                        text.chars().for_each(|c| term.input(c));
                    }
                }
                1 => term.input(['\u{4e16}', '\u{1f600}', '\u{e9}'][arg as usize % 3]),
                2 => term.goto((arg % 6) as i32, (arg / 6 % 13) as usize),
                3 => term.linefeed(),
                4 => term.carriage_return(),
                5 => {
                    if arg % 2 == 0 {
                        term.set_mode(Mode::Named(NamedMode::Insert));
                    } else {
                        term.unset_mode(Mode::Named(NamedMode::Insert));
                    }
                }
                6 => term.configure_charset(
                    CharsetIndex::G0,
                    if arg % 3 == 0 {
                        StandardCharset::SpecialCharacterAndLineDrawing
                    } else {
                        StandardCharset::Ascii
                    },
                ),
                7 => {
                    let wrap = PrivateMode::Named(NamedPrivateMode::LineWrap);
                    if arg % 4 == 0 {
                        term.unset_private_mode(wrap);
                    } else {
                        term.set_private_mode(wrap);
                    }
                }
                _ => {
                    term.set_scrolling_region((arg % 3) as usize + 1, Some((arg % 3) as usize + 4));
                }
            }
        }
        let grid = term.grid();
        let rows: Vec<Vec<Cell>> = (0..grid.screen_lines())
            .map(|line| grid[alacritty_terminal::index::Line(line as i32)][..].to_vec())
            .collect();
        (
            (rows, grid.cursor.point, grid.cursor.input_needs_wrap),
            *term.mode(),
        )
    };
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..200 {
        let ops: Vec<(u8, u32)> = (0..120)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let op = state % 20;
                (
                    (if op < 9 { 0 } else { op - 8 }).min(8) as u8,
                    (state >> 16) as u32,
                )
            })
            .collect();
        let (bulk, single) = (run(&ops, true), run(&ops, false));
        assert_eq!(bulk.0, single.0, "screens differ for {ops:?}");
        assert_eq!(bulk.1, single.1);
    }
}

/// Long runs of text that wrap past the bottom of the screen keep the same
/// scrollback and screen whether written a row at a time or a character at
/// a time.
///
/// Sabotage: in `Hooked::input_ascii`, skip `keep_top` before a wrapping
/// row. The rows that wrapped off the top are then lost from the scrollback.
#[test]
fn wrapped_runs_reach_the_scrollback_as_single_characters_do() {
    let mut text = Vec::new();
    for i in 0..60 {
        text.extend(format!("{i}: ").as_bytes());
        text.extend(std::iter::repeat_n(b'a' + (i % 26) as u8, (i * 7) % 41));
        // Repeat the last character (REP), sometimes past the row's end.
        text.extend(format!("\x1b[{}b", (i * 13) % 57).as_bytes());
        text.extend(if i % 5 == 0 {
            &b"\x1b[31mred\x1b[m\r\n"[..]
        } else if i % 3 == 0 {
            &b"\r\n"[..]
        } else {
            &b" "[..]
        });
    }
    let shown = |single: bool| {
        handler::SINGLE_CHARACTERS.with(|flag| flag.set(single));
        let mut screen = Screen::new(Arc::new(AtomicU64::new(0)));
        screen.resize(5, 11);
        screen.process(&text);
        let rows = screen
            .project("s")
            .into_iter()
            .map(|row| format!("{:?}", *row))
            .collect::<Vec<_>>();
        handler::SINGLE_CHARACTERS.with(|flag| flag.set(false));
        rows
    };
    let whole = shown(false);
    assert!(whole.len() > 5, "rows scrolled off");
    assert_eq!(whole, shown(true));
}

/// A live screen with rows above leads with the newest scrollback rows,
/// oldest first, then the emulator's rows and its cursor; with fewer rows
/// of scrollback than room, the top ones are blank.
///
/// Sabotage: write the emulator's rows from row 0 in `live_write` (drop
/// `above +`). They overwrite the scrollback rows.
#[test]
fn a_live_screen_with_rows_above_leads_with_the_newest_scrollback() {
    let _scheme = crate::color_scheme::steady_scheme();
    let mut s = screen(3, 20);
    s.process(b"line 1\r\nline 2\r\nline 3\r\nline 4\r\nline 5");
    let text = |view: &enzui_terminal::TerminalScreen| -> Vec<String> {
        view.read(|v| (0..v.rows()).map(|y| v.row_text(y)).collect())
    };
    for (above, expected) in [
        (2, &["line 1", "line 2", "line 3", "line 4", "line 5"][..]),
        (
            3,
            &["", "line 1", "line 2", "line 3", "line 4", "line 5"][..],
        ),
    ] {
        let view = enzui_terminal::TerminalScreen::new(1, 1, Default::default());
        s.set_live(Some(
            LiveTarget::new(view.clone(), true, "block").rows_above(above),
        ))
        .expect("starting writes the whole screen")
        .apply();
        assert_eq!(text(&view), expected, "{above} rows above");
        assert_eq!(view.read(|v| v.cursor().row), above + 2, "the cursor's row");
    }
}
