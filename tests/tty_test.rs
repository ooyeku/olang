//! The `tty` module (milestone H0 of Heddle): the input decoder on byte
//! sequences, `str.width`, and the restore guarantee end to end — a real
//! olang program in a pseudo-terminal enters raw mode and the alternate
//! screen, then raises, exits, is sent SIGTERM, or panics, and the
//! terminal must come back: the leave sequences on the wire, and the
//! pty's settings cooked again.
//!
//! The pty is opened with `openpty(3)` through libc (already a Unix
//! dependency), so no pty crate is needed. The restore tests run on Unix;
//! they have been run on macOS. The decoder and width tests are pure.

use olang::stdlib::tty::decode::{Decoder, Input, Key, Mouse};
use olang::{Interpreter, Parser, Value};

// ── the decoder ──────────────────────────────────────────────────────

fn feed(bytes: &[u8]) -> Vec<Input> {
    let mut d = Decoder::new();
    let mut out = d.feed(bytes);
    out.extend(d.flush());
    out
}

fn keys(bytes: &[u8]) -> Vec<String> {
    feed(bytes)
        .into_iter()
        .map(|i| match i {
            Input::Key(k) => k.chord(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn one_key(bytes: &[u8]) -> Key {
    match feed(bytes).as_slice() {
        [Input::Key(k)] => k.clone(),
        other => panic!("expected one key from {bytes:?}, got {other:?}"),
    }
}

#[test]
fn printable_text_is_one_key_per_character() {
    assert_eq!(keys(b"ab"), ["a", "b"]);
    assert_eq!(keys("中é🙂".as_bytes()), ["中", "é", "🙂"]);
    let k = one_key(b"A");
    assert_eq!(
        (k.key.as_str(), k.text.as_str(), k.shift),
        ("A", "A", false)
    );
    assert_eq!(k.chord(), "A");
    let space = one_key(b" ");
    assert_eq!((space.key.as_str(), space.text.as_str()), ("space", " "));
}

#[test]
fn utf8_split_across_reads_waits_for_the_rest() {
    let mut d = Decoder::new();
    let bytes = "中".as_bytes();
    assert!(d.feed(&bytes[..1]).is_empty());
    assert!(d.feed(&bytes[1..2]).is_empty());
    assert_eq!(
        d.feed(&bytes[2..]),
        vec![Input::Key(one_key("中".as_bytes()))]
    );
}

#[test]
fn control_bytes_are_ctrl_keys() {
    assert_eq!(keys(b"\x01\x1a"), ["ctrl+a", "ctrl+z"]);
    assert_eq!(keys(b"\x03"), ["ctrl+c"]);
    assert_eq!(keys(b"\r\t\x7f"), ["enter", "tab", "backspace"]);
    assert_eq!(keys(b"\x00"), ["ctrl+space"]);
    assert_eq!(keys(b"\x1c\x1f"), ["ctrl+\\", "ctrl+_"]);
    let k = one_key(b"\x04");
    assert_eq!((k.key.as_str(), k.ctrl, k.text.as_str()), ("d", true, ""));
}

#[test]
fn escape_alone_waits_then_is_the_escape_key() {
    let mut d = Decoder::new();
    assert!(d.feed(b"\x1b").is_empty(), "a lone ESC is pending");
    assert_eq!(
        d.pending(),
        Some(olang::stdlib::tty::decode::ESC_TIMEOUT_MS)
    );
    assert_eq!(keys_of(d.flush()), ["esc"]);
    assert_eq!(d.pending(), None);
}

#[test]
fn escape_then_a_key_in_time_is_alt() {
    assert_eq!(keys(b"\x1ba"), ["alt+a"]);
    assert_eq!(keys(b"\x1bA"), ["alt+shift+a"]);
    assert_eq!(keys(b"\x1b\r"), ["alt+enter"]);
    assert_eq!(keys(b"\x1b\x01"), ["ctrl+alt+a"]);
    // The ESC arrives alone and the key follows before the timeout.
    let mut d = Decoder::new();
    assert!(d.feed(b"\x1b").is_empty());
    assert_eq!(keys_of(d.feed(b"x")), ["alt+x"]);
    // ESC then `[` alone is alt+[ once the wait is over.
    assert_eq!(keys(b"\x1b["), ["alt+["]);
    assert_eq!(keys(b"\x1b\x1b"), ["alt+esc"]);
}

fn keys_of(inputs: Vec<Input>) -> Vec<String> {
    inputs
        .into_iter()
        .map(|i| match i {
            Input::Key(k) => k.chord(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn cursor_and_editing_keys_in_every_encoding() {
    assert_eq!(
        keys(b"\x1b[A\x1b[B\x1b[C\x1b[D"),
        ["up", "down", "right", "left"]
    );
    assert_eq!(keys(b"\x1bOA\x1bOD"), ["up", "left"]); // application cursor mode
    assert_eq!(
        keys(b"\x1b[H\x1b[F\x1b[1~\x1b[4~"),
        ["home", "end", "home", "end"]
    );
    assert_eq!(
        keys(b"\x1b[2~\x1b[3~\x1b[5~\x1b[6~"),
        ["insert", "delete", "pgup", "pgdown"]
    );
    assert_eq!(keys(b"\x1b[Z"), ["shift+tab"]);
    assert_eq!(keys(b"\x1b[1;5A"), ["ctrl+up"]);
    assert_eq!(keys(b"\x1b[1;3D"), ["alt+left"]);
    assert_eq!(keys(b"\x1b[1;2C"), ["shift+right"]);
    assert_eq!(keys(b"\x1b[3;5~"), ["ctrl+delete"]);
    assert_eq!(keys(b"\x1b[1;12A"), ["alt+shift+super+up"]);
    // alt+up as some terminals send it: ESC ESC [ A.
    assert_eq!(keys(b"\x1b\x1b[A"), ["alt+up"]);
}

#[test]
fn function_keys() {
    assert_eq!(keys(b"\x1bOP\x1bOQ\x1bOR\x1bOS"), ["f1", "f2", "f3", "f4"]);
    assert_eq!(
        keys(b"\x1b[15~\x1b[17~\x1b[21~\x1b[24~"),
        ["f5", "f6", "f10", "f12"]
    );
    assert_eq!(keys(b"\x1b[1;5P"), ["ctrl+f1"]);
    assert_eq!(keys(b"\x1b[1;5R"), ["ctrl+f3"]);
    assert_eq!(keys(b"\x1b[15;2~"), ["shift+f5"]);
    assert_eq!(keys(b"\x1b[[A\x1b[[E"), ["f1", "f5"]); // the Linux console
}

#[test]
fn kitty_keyboard_protocol() {
    assert_eq!(keys(b"\x1b[97;5u"), ["ctrl+a"]);
    assert_eq!(keys(b"\x1b[105;5u"), ["ctrl+i"]); // distinct from tab
    assert_eq!(keys(b"\x1b[9u"), ["tab"]);
    assert_eq!(keys(b"\x1b[9;5u"), ["ctrl+tab"]);
    assert_eq!(keys(b"\x1b[27u"), ["esc"]);
    assert_eq!(keys(b"\x1b[13;2u"), ["shift+enter"]);
    assert_eq!(keys(b"\x1b[97;6u"), ["ctrl+shift+a"]);
    assert_eq!(keys(b"\x1b[97;9u"), ["super+a"]);
    assert_eq!(keys(b"\x1b[127;3u"), ["alt+backspace"]);
    // Keypad and functional code points.
    assert_eq!(keys(b"\x1b[57399u\x1b[57414u"), ["0", "enter"]);
    assert_eq!(keys(b"\x1b[57376u"), ["f13"]);
    assert_eq!(keys(b"\x1b[57441;2u"), ["shift+left_shift"]);
    // Event types: a repeat is a press, a release is dropped.
    assert_eq!(keys(b"\x1b[97;1:2u"), ["a"]);
    assert!(feed(b"\x1b[97;1:3u").is_empty());
    // The text field (flag 16), when the terminal sends it.
    let k = one_key(b"\x1b[97;2;65u");
    assert_eq!(k.text, "A");
    // Legacy keys with kitty modifiers still decode.
    assert_eq!(keys(b"\x1b[1;5A"), ["ctrl+up"]);
    // xterm modifyOtherKeys.
    assert_eq!(keys(b"\x1b[27;5;105~"), ["ctrl+i"]);
}

#[test]
fn bracketed_paste_is_one_event() {
    assert_eq!(
        feed(b"\x1b[200~hello\r\nworld\rx\x1b[201~"),
        vec![Input::Paste("hello\nworld\nx".to_string())]
    );
    // What looks like keys inside a paste is text.
    assert_eq!(
        feed(b"\x1b[200~\x1b[A\x03q\x1b[201~"),
        vec![Input::Paste("\x1b[A\x03q".to_string())]
    );
    // Split across reads, the end marker included, and never flushed early.
    let mut d = Decoder::new();
    assert!(d.feed(b"\x1b[200~ab").is_empty());
    assert_eq!(d.pending(), None, "a paste waits for its end however long");
    assert!(d.flush().is_empty());
    assert!(d.feed(b"c\x1b[20").is_empty());
    assert_eq!(
        d.feed(b"1~z"),
        vec![Input::Paste("abc".into()), Input::Key(one_key(b"z"))]
    );
}

fn mouse(bytes: &[u8]) -> Mouse {
    match feed(bytes).as_slice() {
        [Input::Mouse(m)] => m.clone(),
        other => panic!("expected one mouse event from {bytes:?}, got {other:?}"),
    }
}

#[test]
fn sgr_mouse_reports() {
    let m = mouse(b"\x1b[<0;11;6M");
    assert_eq!((m.action, m.button, m.x, m.y), ("down", "left", 10, 5));
    let m = mouse(b"\x1b[<0;11;6m");
    assert_eq!((m.action, m.button), ("up", "left"));
    let m = mouse(b"\x1b[<2;1;1M");
    assert_eq!((m.action, m.button, m.x, m.y), ("down", "right", 0, 0));
    let m = mouse(b"\x1b[<32;5;5M");
    assert_eq!((m.action, m.button), ("drag", "left"));
    let m = mouse(b"\x1b[<35;300;200M");
    assert_eq!((m.action, m.button, m.x, m.y), ("move", "none", 299, 199));
    let m = mouse(b"\x1b[<64;1;1M");
    assert_eq!((m.action, m.button), ("wheel", "up"));
    let m = mouse(b"\x1b[<65;1;1M");
    assert_eq!((m.action, m.button), ("wheel", "down"));
    let m = mouse(b"\x1b[<16;1;1M");
    assert!(m.ctrl && !m.alt && !m.shift);
    let m = mouse(b"\x1b[<12;1;1M");
    assert!(m.shift && m.alt && !m.ctrl);
}

#[test]
fn x10_mouse_reports() {
    let m = mouse(&[0x1b, b'[', b'M', 32, 33 + 4, 33 + 2]);
    assert_eq!((m.action, m.button, m.x, m.y), ("down", "left", 4, 2));
    let m = mouse(&[0x1b, b'[', b'M', 32 + 3, 33, 33]);
    assert_eq!((m.action, m.button), ("up", "none"));
}

#[test]
fn focus_reports() {
    assert_eq!(
        feed(b"\x1b[I\x1b[O"),
        vec![Input::Focus(true), Input::Focus(false)]
    );
}

#[test]
fn replies_are_never_keys() {
    let osc = b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\";
    assert_eq!(
        feed(osc),
        vec![Input::Reply(String::from_utf8(osc.to_vec()).unwrap())]
    );
    let osc_bel = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
    assert!(matches!(feed(osc_bel).as_slice(), [Input::Reply(_)]));
    assert_eq!(
        feed(b"\x1b[?62;22c"),
        vec![Input::Reply("\x1b[?62;22c".into())]
    );
    assert_eq!(
        feed(b"\x1b[>1;4000;29c"),
        vec![Input::Reply("\x1b[>1;4000;29c".into())]
    );
    assert_eq!(feed(b"\x1b[?1u"), vec![Input::Reply("\x1b[?1u".into())]); // kitty flags
    assert_eq!(
        feed(b"\x1b[?2026;2$y"),
        vec![Input::Reply("\x1b[?2026;2$y".into())]
    );
    assert!(matches!(
        feed(b"\x1bP>|kitty(0.35)\x1b\\").as_slice(),
        [Input::Reply(_)]
    ));
    assert!(matches!(
        feed(b"\x1b[8;24;80t").as_slice(),
        [Input::Reply(_)]
    ));
    // A cursor position report is a reply; F3 with modifiers is a key.
    assert!(matches!(feed(b"\x1b[12;40R").as_slice(), [Input::Reply(_)]));
    let mut d = Decoder::new();
    d.expect_cpr = true;
    assert!(matches!(d.feed(b"\x1b[1;1R").as_slice(), [Input::Reply(_)]));
    // A reply split across reads waits for its end.
    let mut d = Decoder::new();
    assert!(d.feed(b"\x1b]11;rgb:00").is_empty());
    assert_eq!(
        d.pending(),
        Some(olang::stdlib::tty::decode::SEQUENCE_TIMEOUT_MS)
    );
    assert!(matches!(
        d.feed(b"00/0000/0000\x07a").as_slice(),
        [Input::Reply(_), Input::Key(_)]
    ));
    // A reply amid typing leaves the typing alone.
    assert_eq!(
        keys(b"a\x1b[?62c b"),
        ["a", "Reply(\"\\u{1b}[?62c\")", "space", "b"]
    );
}

#[test]
fn unknown_sequences_are_dropped_not_typed() {
    assert!(feed(b"\x1b[99x").is_empty());
    assert_eq!(keys(b"\x1b[99xq"), ["q"]);
}

#[test]
fn event_maps_have_the_documented_shapes() {
    use olang::stdlib::tty::input_value;
    let get = |v: &Value, k: &str| match v {
        Value::Map(m) => m.get(k).cloned().unwrap_or(Value::Unit),
        other => panic!("not a map: {other:?}"),
    };
    let s = |t: &str| Value::String(std::sync::Arc::new(t.to_string()));
    let key = input_value(&feed(b"\x1b[1;5A")[0]);
    assert_eq!(get(&key, "kind"), s("key"));
    assert_eq!(get(&key, "key"), s("up"));
    assert_eq!(get(&key, "text"), s(""));
    assert_eq!(get(&key, "chord"), s("ctrl+up"));
    for (m, v) in [
        ("ctrl", true),
        ("alt", false),
        ("shift", false),
        ("super", false),
    ] {
        assert_eq!(get(&key, m), Value::Boolean(v), "{m}");
    }
    let paste = input_value(&Input::Paste("x".into()));
    assert_eq!(
        (get(&paste, "kind"), get(&paste, "text")),
        (s("paste"), s("x"))
    );
    let m = input_value(&feed(b"\x1b[<16;3;4M")[0]);
    assert_eq!(get(&m, "kind"), s("mouse"));
    assert_eq!(get(&m, "action"), s("down"));
    assert_eq!(get(&m, "button"), s("left"));
    assert_eq!(
        (get(&m, "x"), get(&m, "y")),
        (Value::Integer(2), Value::Integer(3))
    );
    assert_eq!(get(&get(&m, "mods"), "ctrl"), Value::Boolean(true));
    let f = input_value(&Input::Focus(false));
    assert_eq!(
        (get(&f, "kind"), get(&f, "on")),
        (s("focus"), Value::Boolean(false))
    );
}

// ── str.width ────────────────────────────────────────────────────────

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

fn width(s: &str) -> usize {
    olang::stdlib::string::display_width(s)
}

#[test]
fn width_counts_cells_not_bytes_or_code_points() {
    assert_eq!(width(""), 0);
    assert_eq!(width("hello"), 5);
    // CJK and fullwidth: two cells each.
    assert_eq!(width("中文字"), 6);
    assert_eq!(width("ｈｉ"), 4);
    assert_eq!(width("한국어"), 6);
    // Hangul spelled as conjoining jamo is one cluster, two cells.
    assert_eq!(width("\u{1100}\u{1161}\u{11A8}"), 2);
    // Combining marks ride on their base.
    assert_eq!(width("e\u{301}"), 1);
    assert_eq!(width("Z\u{335}\u{321}a\u{300}"), 2);
    // Emoji: presentation, modifiers, ZWJ sequences, flags, keycaps.
    assert_eq!(width("🙂"), 2);
    assert_eq!(width("👍🏽"), 2);
    assert_eq!(width("👨\u{200D}👩\u{200D}👧\u{200D}👦"), 2);
    assert_eq!(width("🏳\u{FE0F}\u{200D}🌈"), 2);
    assert_eq!(width("🇯🇵🇺🇸"), 4);
    assert_eq!(width("#\u{FE0F}\u{20E3}"), 2);
    assert_eq!(width("❤\u{FE0F}"), 2); // emoji presentation sequence
    assert_eq!(width("❤"), 1); // text by default
    // Zero width: controls, joiners, format characters.
    assert_eq!(width("a\tb\nc\x1b"), 3);
    assert_eq!(width("\r\n"), 0);
    assert_eq!(width("\u{200B}\u{200D}\u{AD}"), 0);
    // Mixed.
    assert_eq!(width("ab中🙂e\u{301}"), 7);
}

#[test]
fn width_and_cell_width_from_olang() {
    assert_eq!(eval(r#"str.width("中文 ok")"#), Value::Integer(7));
    assert_eq!(eval(r#"str.cell_width("🇺🇸")"#), Value::Integer(2));
    assert_eq!(eval(r#"str.cell_width("a")"#), Value::Integer(1));
    assert_eq!(eval(r#"str.cell_width("\u{301}")"#), Value::Integer(0));
    assert_eq!(eval(r#"str.cell_width("")"#), Value::Integer(0));
    // The sum over graphemes is the width.
    assert_eq!(
        eval(
            r#"let s = "x中👨‍👩‍👧y"
            col.sum(map(str.graphemes(s), (g) => str.cell_width(g))) == str.width(s)"#
        ),
        Value::Boolean(true)
    );
    // More than one grapheme is a misuse.
    let program = Parser::new().parse(r#"str.cell_width("ab")"#).unwrap();
    assert!(Interpreter::new().eval_program(program).is_err());
}

// ── the module without a terminal ────────────────────────────────────

#[test]
fn off_a_terminal_everything_says_so() {
    // Under `cargo test`, stdout is captured (a pipe), so there is no
    // terminal: enter and size answer Err, is_tty is false.
    if olang::stdlib::tty::entered() {
        return;
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("eval")
        .arg(r#"println(tty.is_tty(), tty.size(), tty.enter(#{ "alt_screen": true }))"#)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("false"), "{text}");
    assert!(
        text.contains(r#"Err("tty.size: stdout is not a terminal")"#),
        "{text}"
    );
    assert!(
        text.contains(r#"Err("tty.enter: stdin is not a terminal")"#),
        "{text}"
    );
    // An unknown option is a misuse, and raises.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("eval")
        .arg(r#"tty.enter(#{ "alt": true })"#)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option \"alt\""));
}

// ── the restore guarantee, in a pseudo-terminal ──────────────────────

#[cfg(unix)]
mod pty {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// The enter sequence the programs below turn on, and the leave
    /// sequences that must follow whatever happens.
    const ALT_ON: &str = "\x1b[?1049h";
    const ALT_OFF: &str = "\x1b[?1049l";
    const CURSOR_ON: &str = "\x1b[?25h";
    const MOUSE_OFF: &str = "\x1b[?1000l";
    const PASTE_OFF: &str = "\x1b[?2004l";

    pub struct Pty {
        master: libc::c_int,
        slave: libc::c_int,
        pub out: Vec<u8>,
        child: Child,
        cooked: libc::termios,
        _dir: tempfile::TempDir,
    }

    fn termios(fd: libc::c_int) -> libc::termios {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(fd, &mut t) }, 0);
        t
    }

    /// The flags raw mode changes; restored means all of them match.
    fn modes(t: &libc::termios) -> (libc::tcflag_t, libc::tcflag_t, libc::tcflag_t) {
        let l = libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN;
        let i = libc::ICRNL | libc::IXON;
        (t.c_lflag & l, t.c_iflag & i, t.c_oflag & libc::OPOST)
    }

    impl Pty {
        /// Run `olang <program>` on a fresh 80x24 pty as its controlling
        /// terminal, with `env` added.
        pub fn spawn(program: &str, env: &[(&str, &str)]) -> Pty {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("prog.ol");
            std::fs::File::create(&path)
                .unwrap()
                .write_all(program.as_bytes())
                .unwrap();
            let (mut master, mut slave) = (0, 0);
            let ws = libc::winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let r = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &ws as *const libc::winsize as *mut libc::winsize,
                )
            };
            assert_eq!(r, 0, "openpty");
            let cooked = termios(slave);
            assert_ne!(modes(&cooked).0 & libc::ICANON, 0, "a fresh pty is cooked");
            let io = |fd: libc::c_int| unsafe { Stdio::from_raw_fd(libc::dup(fd)) };
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
            cmd.arg(&path)
                .stdin(io(slave))
                .stdout(io(slave))
                .stderr(io(slave))
                .env("NO_COLOR", "1")
                .env_remove("OLANG_TTY_TEST_PANIC");
            for (k, v) in env {
                cmd.env(k, v);
            }
            unsafe {
                cmd.pre_exec(|| {
                    // A session of its own, with the pty as its controlling
                    // terminal: job control and SIGWINCH as in a shell.
                    if libc::setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = cmd.spawn().unwrap();
            Pty {
                master,
                slave,
                out: Vec::new(),
                child,
                cooked,
                _dir: dir,
            }
        }

        fn pump(&mut self, wait: Duration) {
            let mut buf = [0u8; 4096];
            let mut fds = [libc::pollfd {
                fd: self.master,
                events: libc::POLLIN,
                revents: 0,
            }];
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, wait.as_millis() as libc::c_int) };
            if n > 0 {
                let r = unsafe {
                    libc::read(
                        self.master,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if r > 0 {
                    self.out.extend_from_slice(&buf[..r as usize]);
                }
            }
        }

        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.out).into_owned()
        }

        /// Read until the output contains `needle`.
        pub fn expect(&mut self, needle: &str) {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !self.text().contains(needle) {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {needle:?}; output so far: {:?}",
                    self.text()
                );
                self.pump(Duration::from_millis(50));
            }
        }

        pub fn send(&mut self, bytes: &[u8]) {
            let n = unsafe {
                libc::write(
                    self.master,
                    bytes.as_ptr() as *const libc::c_void,
                    bytes.len(),
                )
            };
            assert_eq!(n, bytes.len() as isize);
        }

        pub fn signal(&mut self, sig: libc::c_int) {
            unsafe {
                libc::kill(self.child.id() as libc::pid_t, sig);
            }
        }

        pub fn resize(&mut self, cols: u16, rows: u16) {
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe {
                libc::ioctl(self.master, libc::TIOCSWINSZ as _, &ws);
            }
        }

        /// Wait until the child is stopped (by a job-control signal).
        pub fn wait_stopped(&mut self) -> bool {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                let mut status = 0;
                let r = unsafe {
                    libc::waitpid(
                        self.child.id() as libc::pid_t,
                        &mut status,
                        libc::WUNTRACED | libc::WNOHANG,
                    )
                };
                if r > 0 && libc::WIFSTOPPED(status) {
                    return true;
                }
                self.pump(Duration::from_millis(20));
            }
            false
        }

        /// While the program runs: is the pty in raw mode now?
        pub fn is_raw(&self) -> bool {
            modes(&termios(self.master)).0 & libc::ICANON == 0
        }

        /// Wait for the child to end (collecting its output) and return
        /// its status.
        pub fn finish(&mut self) -> std::process::ExitStatus {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                self.pump(Duration::from_millis(20));
                if let Some(st) = self.child.try_wait().unwrap() {
                    // Drain what is left.
                    for _ in 0..10 {
                        self.pump(Duration::from_millis(20));
                    }
                    return st;
                }
                if Instant::now() > deadline {
                    let _ = self.child.kill();
                    panic!("the program did not end; output: {:?}", self.text());
                }
            }
        }

        /// The restore guarantee: every mode the program turned on was
        /// turned off after it was turned on, and the pty is cooked again
        /// exactly as it was.
        pub fn assert_restored(&self) {
            let text = self.text();
            let on = text
                .find(ALT_ON)
                .expect("the program entered the alternate screen");
            for seq in [ALT_OFF, CURSOR_ON, MOUSE_OFF, PASTE_OFF] {
                let off = text
                    .rfind(seq)
                    .unwrap_or_else(|| panic!("{seq:?} never written; output: {text:?}"));
                assert!(off > on, "{seq:?} came before the enter; output: {text:?}");
            }
            // The master reads the pty's settings too, and still can after
            // the session leader exits (the slave is revoked on macOS).
            let after = termios(self.master);
            assert_eq!(modes(&after), modes(&self.cooked), "termios not restored");
        }

        /// Where `needle` first appears, and where the screen was left.
        pub fn left_before(&self, needle: &str) -> bool {
            let text = self.text();
            match (text.find(ALT_OFF), text.find(needle)) {
                (Some(off), Some(at)) => off < at,
                _ => false,
            }
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            unsafe {
                libc::close(self.master);
                libc::close(self.slave);
            }
        }
    }
}

/// The program every restore test runs: enter everything, say READY,
/// then act on the first key — `r` raises, `x` calls os.exit(7), `p`
/// writes the panic trigger, `q` just ends — or wait (for a signal).
#[cfg(unix)]
const VICTIM: &str = r#"
let h = unwrap(tty.enter(#{ "alt_screen": true, "hide_cursor": true, "mouse": true, "paste": true, "focus": true, "kitty_keys": true }))
let events = tty.events(h)
tty.write(h, "READY\r\n")
let mut going = true
while going {
    match chan.recv(events) {
        Ok(ev) => {
            let k = map_get(ev, "chord")
            tty.write(h, "GOT " + show(map_get(ev, "kind")) + " " + show(k) + "\r\n")
            if k == "r" => { unwrap(Err("raised on purpose")) }
            else if k == "x" => { os.exit(7) }
            else if k == "p" => { tty.write(h, "PANIC-NOW") }
            else if k == "q" => { going = false }
        },
        Err(e) => { going = false }
    }
}
"#;

#[cfg(unix)]
#[test]
fn restored_after_an_uncaught_raise_before_the_error_prints() {
    let mut p = pty::Pty::spawn(VICTIM, &[]);
    p.expect("READY");
    assert!(p.is_raw(), "the program is in raw mode");
    p.send(b"r");
    let st = p.finish();
    assert_eq!(st.code(), Some(1));
    p.assert_restored();
    assert!(p.text().contains("raised on purpose"));
    assert!(
        p.left_before("raised on purpose"),
        "the terminal is restored before the error prints: {:?}",
        p.text()
    );
}

#[cfg(unix)]
#[test]
fn restored_after_os_exit() {
    let mut p = pty::Pty::spawn(VICTIM, &[]);
    p.expect("READY");
    p.send(b"x");
    let st = p.finish();
    assert_eq!(st.code(), Some(7));
    p.assert_restored();
}

#[cfg(unix)]
#[test]
fn restored_when_the_program_ends_without_leaving() {
    let mut p = pty::Pty::spawn(VICTIM, &[]);
    p.expect("READY");
    p.send(b"q");
    let st = p.finish();
    assert_eq!(st.code(), Some(0));
    p.assert_restored();
}

#[cfg(unix)]
#[test]
fn restored_after_sigterm_and_the_process_dies_of_it() {
    use std::os::unix::process::ExitStatusExt;
    let mut p = pty::Pty::spawn(VICTIM, &[]);
    p.expect("READY");
    assert!(p.is_raw());
    p.signal(libc::SIGTERM);
    let st = p.finish();
    assert_eq!(
        st.signal(),
        Some(libc::SIGTERM),
        "dies of SIGTERM as by default"
    );
    p.assert_restored();
}

#[cfg(unix)]
#[test]
fn restored_after_sighup_and_sigint() {
    use std::os::unix::process::ExitStatusExt;
    for sig in [libc::SIGHUP, libc::SIGINT] {
        let mut p = pty::Pty::spawn(VICTIM, &[]);
        p.expect("READY");
        p.signal(sig);
        let st = p.finish();
        assert_eq!(st.signal(), Some(sig));
        p.assert_restored();
    }
}

/// A panic needs a way to cause one: debug builds of olang panic when a
/// program writes the text named by `OLANG_TTY_TEST_PANIC`. Release
/// builds (panic = "abort") run the same hook before aborting.
#[cfg(all(unix, debug_assertions))]
#[test]
fn restored_after_a_panic_before_the_panic_message() {
    let mut p = pty::Pty::spawn(VICTIM, &[("OLANG_TTY_TEST_PANIC", "PANIC-NOW")]);
    p.expect("READY");
    p.send(b"p");
    let st = p.finish();
    assert!(!st.success());
    p.assert_restored();
    assert!(p.text().contains("forced panic"), "{:?}", p.text());
    assert!(p.left_before("forced panic"), "{:?}", p.text());
}

#[cfg(unix)]
#[test]
fn a_trapped_sigterm_is_an_event_and_the_program_decides() {
    let program = r#"
os.on_interrupt()
let h = unwrap(tty.enter(#{ "alt_screen": true, "mouse": true, "paste": true }))
let events = tty.events(h)
tty.write(h, "READY\r\n")
let mut going = true
while going {
    match chan.recv(events) {
        Ok(ev) => {
            if map_get(ev, "kind") == "signal" => {
                tty.write(h, "SIGNAL " + map_get(ev, "name") + "\r\n")
                going = false
            }
        },
        Err(e) => { going = false }
    }
}
tty.leave(h)
println("LEFT " + show(os.interrupted()))
"#;
    let mut p = pty::Pty::spawn(program, &[]);
    p.expect("READY");
    p.signal(libc::SIGTERM);
    p.expect("SIGNAL term");
    let st = p.finish();
    assert_eq!(st.code(), Some(0), "{:?}", p.text());
    p.assert_restored();
    assert!(p.text().contains("LEFT true"), "{:?}", p.text());
}

#[cfg(unix)]
#[test]
fn events_arrive_decoded_through_a_real_terminal() {
    let program = r#"
let h = unwrap(tty.enter(#{ "alt_screen": true, "mouse": true, "paste": true, "focus": true }))
let events = tty.events(h)
tty.write(h, "READY\r\n")
let mut n = 0
while n < 7 {
    match chan.recv(events) {
        Ok(ev) => {
            let kind = map_get(ev, "kind")
            let what = match kind {
                "key" => map_get(ev, "chord"),
                "paste" => map_get(ev, "text"),
                "mouse" => map_get(ev, "action") + ":" + map_get(ev, "button") + ":" + show(map_get(ev, "x")) + "," + show(map_get(ev, "y")),
                "resize" => show(map_get(ev, "cols")) + "x" + show(map_get(ev, "rows")),
                "focus" => show(map_get(ev, "on")),
                other => show(ev)
            }
            tty.write(h, "EV " + kind + " " + what + ";\r\n")
            n = n + 1
        },
        Err(e) => { n = 99 }
    }
}
tty.leave(h)
"#;
    let mut p = pty::Pty::spawn(program, &[]);
    p.expect("READY");
    p.send(b"a");
    p.expect("EV key a;");
    p.send(b"\x1b[1;5A");
    p.expect("EV key ctrl+up;");
    p.send(b"\x1b");
    p.expect("EV key esc;"); // after the ESC timeout
    p.send(b"\x1b[200~pasted text\x1b[201~");
    p.expect("EV paste pasted text;");
    p.send(b"\x1b[<0;5;3M");
    p.expect("EV mouse down:left:4,2;");
    p.send(b"\x1b[I");
    p.expect("EV focus true;");
    p.resize(100, 30);
    p.expect("EV resize 100x30;");
    let st = p.finish();
    assert!(st.success(), "{:?}", p.text());
    p.assert_restored();
}

#[cfg(unix)]
#[test]
fn query_replies_come_back_and_never_leak_into_events() {
    let program = r#"
let h = unwrap(tty.enter(#{ "alt_screen": true, "mouse": true, "paste": true }))
let events = tty.events(h)
tty.write(h, "READY\r\n")
let r = tty.query(h, "\u{1b}[c", 5000)
tty.write(h, "REPLY " + show(unwrap_or(r, "") == "\u{1b}[?62;22c") + "\r\n")
let t = tty.query(h, "\u{1b}]11;?\u{1b}\\", 200)
tty.write(h, "TIMEOUT " + show(t) + "\r\n")
match chan.recv(events) {
    Ok(ev) => tty.write(h, "NEXT " + map_get(ev, "kind") + " " + show(map_get(ev, "chord")) + "\r\n"),
    Err(e) => ()
}
tty.leave(h)
"#;
    let mut p = pty::Pty::spawn(program, &[]);
    p.expect("READY");
    p.expect("\x1b[c");
    p.send(b"\x1b[?62;22c");
    p.expect("REPLY true");
    p.expect("TIMEOUT Err(\"timeout\")");
    // A late reply after the timeout, then a key: the key is the next event.
    p.send(b"\x1b]11;rgb:0000/0000/0000\x1b\\");
    std::thread::sleep(std::time::Duration::from_millis(200));
    p.send(b"k");
    p.expect("NEXT key k");
    let st = p.finish();
    assert!(st.success(), "{:?}", p.text());
    p.assert_restored();
}

#[cfg(unix)]
#[test]
fn leave_restores_and_a_second_enter_works() {
    let program = r#"
let h = unwrap(tty.enter(#{ "alt_screen": true, "mouse": true, "paste": true }))
println(tty.enter(#{}))
tty.leave(h)
tty.leave(h)
println(tty.write(h, "x"))
let h2 = unwrap(tty.enter(#{ "alt_screen": true, "mouse": true, "paste": true }))
tty.write(h2, "SECOND\r\n")
tty.leave(h2)
println("DONE")
"#;
    let mut p = pty::Pty::spawn(program, &[]);
    p.expect("DONE");
    let st = p.finish();
    assert!(st.success(), "{:?}", p.text());
    let text = p.text();
    assert!(text.contains("the terminal is already entered"), "{text:?}");
    assert!(
        text.contains("the handle has left the terminal"),
        "{text:?}"
    );
    assert_eq!(text.matches("\x1b[?1049h").count(), 2);
    assert_eq!(
        text.matches("\x1b[?1049l").count(),
        2,
        "leave is idempotent"
    );
    p.assert_restored();
}

#[cfg(unix)]
#[test]
fn suspend_restores_stops_and_resumes_on_sigcont() {
    let program = r#"
let h = unwrap(tty.enter(#{ "alt_screen": true, "mouse": true, "paste": true }))
let events = tty.events(h)
tty.write(h, "READY\r\n")
let mut going = true
while going {
    match chan.recv(events) {
        Ok(ev) => {
            let kind = map_get(ev, "kind")
            if kind == "signal" => tty.write(h, "SIGNAL " + map_get(ev, "name") + "\r\n")
            else if kind == "key" && map_get(ev, "chord") == "z" => {
                tty.suspend(h)
                tty.write(h, "RESUMED\r\n")
            }
            else if kind == "key" && map_get(ev, "chord") == "q" => { going = false }
        },
        Err(e) => { going = false }
    }
}
tty.leave(h)
"#;
    let mut p = pty::Pty::spawn(program, &[]);
    p.expect("READY");
    assert!(p.is_raw());
    p.send(b"z");
    p.expect("\x1b[?1049l");
    assert!(p.wait_stopped(), "the process stops");
    assert!(!p.is_raw(), "stopped with the terminal cooked");
    p.signal(libc::SIGCONT);
    p.expect("RESUMED");
    p.expect("SIGNAL cont");
    assert!(p.is_raw(), "raw again after SIGCONT");
    assert_eq!(
        p.text().matches("\x1b[?1049h").count(),
        2,
        "re-entered the alternate screen"
    );
    p.send(b"q");
    let st = p.finish();
    assert!(st.success(), "{:?}", p.text());
    p.assert_restored();
}
