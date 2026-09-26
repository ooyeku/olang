//! The terminal input decoder: bytes from a terminal in, events out.
//!
//! A pure state machine with no I/O, so every sequence it understands is
//! pinned by a test that feeds it bytes (`tests/tty_test.rs`). The Unix
//! reader thread feeds it whatever `read(2)` returned; on Windows the
//! console delivers structured input records instead and this decoder is
//! not used (see `windows.rs`).
//!
//! Why not crossterm's parser on Unix: it is private (it cannot be fed
//! bytes by a test), it consumes the terminal's replies to queries as its
//! own internal events or as garbage keys (so `tty.query` could not be
//! built on it), and it has no notion of "wait a moment before deciding a
//! lone ESC was the Escape key".
//!
//! What it understands:
//!
//! - UTF-8 text, one key event per character; control bytes as ctrl+key
//!   (`0x01` is ctrl+a, `0x7f` backspace, `\r` enter, `\t` tab);
//! - `ESC x` as alt+x (any key, including `ESC ESC [ A` for alt+up);
//! - CSI and SS3 keys in the xterm encoding, modifiers in the second
//!   parameter (`ESC [ 1 ; 5 A` is ctrl+up), the `~` keys (insert,
//!   delete, home, end, page up/down, f1–f20), xterm's modifyOtherKeys
//!   (`ESC [ 27 ; m ; code ~`), and shift+tab (`ESC [ Z`);
//! - the kitty keyboard protocol (`ESC [ code[:shifted[:base]] ; mods[:event] [; text] u`),
//!   including its functional-key code points; release events are dropped;
//! - bracketed paste (`ESC [ 200 ~ … ESC [ 201 ~`) as one event, CR and
//!   CRLF normalised to LF;
//! - SGR mouse (`ESC [ < b ; x ; y M/m`) and the X10 form (`ESC [ M b x y`);
//! - focus in and out (`ESC [ I`, `ESC [ O`);
//! - terminal *replies* — OSC, DCS, APC, PM and SOS strings, CSI
//!   sequences with a private prefix (`?`, `>`, `=`) or an intermediate
//!   byte (DA1, DA2, DECRPM, the kitty flags reply), cursor position
//!   reports while one is expected, `CSI … t`, `CSI … n`, and `CSI … c`.
//!   These are returned as [`Input::Reply`] and never as keys.
//!
//! **ESC and alt.** A lone ESC, or ESC followed by one of the bytes that
//! open a sequence (`[`, `O`, `]`, `P`, `_`), is ambiguous until more
//! bytes arrive or a moment passes. [`Decoder::feed`] keeps such a tail
//! pending; the reader waits [`ESC_TIMEOUT_MS`] (a longer
//! [`SEQUENCE_TIMEOUT_MS`] for a longer, clearly-a-sequence tail) and
//! then calls [`Decoder::flush`], which resolves it as typed keys: the
//! Escape key, or alt+the next key.

/// How long a lone ESC (or `ESC [`, `ESC O`, …) waits for the rest of a
/// sequence before it is the Escape key (or alt+`[`, …). Terminals write
/// a whole sequence at once, so the rest arrives in the same read almost
/// always; 25 ms is below what a person notices after pressing Escape.
pub const ESC_TIMEOUT_MS: u64 = 25;

/// How long a partial sequence of three bytes or more waits before it is
/// given up on and resolved as keys: long enough for a reply split across
/// packets on a remote session.
pub const SEQUENCE_TIMEOUT_MS: u64 = 250;

/// A decoded key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// The key's name: the character itself for a printable key (`"a"`,
    /// `"A"`, `"中"`, `"?"`), or one of `space enter tab backspace esc up
    /// down left right home end pgup pgdown insert delete begin f1…f35`,
    /// keypad and modifier keys under the kitty protocol (`left_shift`, …).
    pub key: String,
    /// What the key types: the character for a printable key without
    /// ctrl, alt or super; `""` otherwise.
    pub text: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
}

impl Key {
    fn named(name: &str) -> Key {
        Key {
            key: name.to_string(),
            text: String::new(),
            ctrl: false,
            alt: false,
            shift: false,
            super_: false,
        }
    }

    fn char(c: char) -> Key {
        if c == ' ' {
            let mut k = Key::named("space");
            k.text = " ".to_string();
            return k;
        }
        Key {
            key: c.to_string(),
            text: c.to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            super_: false,
        }
    }

    fn with_mods(mut self, m: Mods) -> Key {
        self.ctrl |= m.ctrl;
        self.alt |= m.alt;
        self.shift |= m.shift;
        self.super_ |= m.super_;
        self.fix_text();
        self
    }

    fn with_alt(mut self) -> Key {
        self.alt = true;
        self.fix_text();
        self
    }

    fn fix_text(&mut self) {
        if self.ctrl || self.alt || self.super_ {
            self.text.clear();
        }
    }

    /// The chord this key is, in one spelling: modifiers in the order
    /// ctrl, alt, shift, super, then the key (`"ctrl+d"`, `"alt+enter"`,
    /// `"shift+tab"`). A capital letter typed plainly is itself (`"G"`);
    /// with ctrl or alt it is spelled with shift (`"ctrl+shift+g"`).
    /// This is the spelling Heddle's key table parses to.
    pub fn chord(&self) -> String {
        let mut key = self.key.clone();
        let mut shift = self.shift;
        let upper_letter = key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase();
        let lower_letter = key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase();
        if upper_letter && (self.ctrl || self.alt || self.super_) {
            key = key.to_ascii_lowercase();
            shift = true;
        } else if upper_letter {
            shift = false;
        } else if lower_letter && shift && !self.ctrl && !self.alt && !self.super_ {
            key = key.to_ascii_uppercase();
            shift = false;
        } else if self.text.chars().count() == 1 && self.key != "space" && !self.ctrl && !self.alt {
            // A printable symbol already carries its shift (`?`, `!`).
            shift = false;
        }
        let mut out = String::new();
        if self.ctrl {
            out.push_str("ctrl+");
        }
        if self.alt {
            out.push_str("alt+");
        }
        if shift {
            out.push_str("shift+");
        }
        if self.super_ {
            out.push_str("super+");
        }
        out.push_str(&key);
        out
    }
}

/// A mouse report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mouse {
    /// `"down"`, `"up"`, `"move"` (no button held), `"drag"` (a button
    /// held), or `"wheel"`.
    pub action: &'static str,
    /// `"left"`, `"middle"`, `"right"`, `"none"` (a move, or a release
    /// the encoding does not attribute), `"up"`/`"down"`/`"left"`/`"right"`
    /// for the wheel, or `"b8"`…`"b11"` for extra buttons.
    pub button: &'static str,
    /// Zero-based cell column and row.
    pub x: u32,
    pub y: u32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// One decoded input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Key(Key),
    /// One bracketed paste, whole.
    Paste(String),
    Mouse(Mouse),
    /// The terminal window gained (`true`) or lost focus.
    Focus(bool),
    /// A reply to a query, verbatim (escape bytes included). Never an event.
    Reply(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Mods {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_: bool,
}

/// The xterm modifier parameter: 1 + a bit set (shift 1, alt 2, ctrl 4,
/// super 8; kitty's hyper 16 and meta 32 and the lock bits are ignored).
fn mods_of(param: u32) -> Mods {
    let bits = param.saturating_sub(1);
    Mods {
        shift: bits & 1 != 0,
        alt: bits & 2 != 0,
        ctrl: bits & 4 != 0,
        super_: bits & 8 != 0,
    }
}

const ESC: u8 = 0x1b;

/// The decoder's state: bytes not yet resolved, and a bracketed paste in
/// progress.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    paste: Option<Vec<u8>>,
    /// Set while a query waits for its answer: a cursor position report
    /// (`ESC [ row ; col R`) is then a reply, not the key f3.
    pub expect_cpr: bool,
}

/// What parsing the front of the buffer produced.
enum Step {
    /// `n` bytes consumed, with an input (or nothing: an unknown or
    /// dropped sequence).
    Done(usize, Option<Input>),
    /// The front is a prefix of something longer.
    Incomplete,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Feed bytes; returns every input they complete. A tail that could
    /// still become a longer sequence stays pending (see [`Decoder::pending`]).
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Input> {
        self.buf.extend_from_slice(bytes);
        self.drain(false)
    }

    /// Whether bytes are waiting on more input to decide what they are.
    /// `Some(ms)` is how long the reader should wait before calling
    /// [`Decoder::flush`]; `None` means nothing is pending (a bracketed
    /// paste in progress waits for its end marker however long it takes).
    pub fn pending(&self) -> Option<u64> {
        if self.paste.is_some() || self.buf.is_empty() {
            return None;
        }
        Some(if self.buf.len() <= 2 {
            ESC_TIMEOUT_MS
        } else {
            SEQUENCE_TIMEOUT_MS
        })
    }

    /// The wait is over: resolve the pending tail as typed keys (a lone
    /// ESC is the Escape key; `ESC x` is alt+x; a sequence cut short
    /// yields alt+its second byte and then the rest as keys).
    pub fn flush(&mut self) -> Vec<Input> {
        self.drain(true)
    }

    fn drain(&mut self, flush: bool) -> Vec<Input> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        loop {
            if let Some(mut paste) = self.paste.take() {
                // Inside a paste: everything up to the end marker is text.
                const END: &[u8] = b"\x1b[201~";
                let rest = &self.buf[pos..];
                match find(rest, END) {
                    Some(at) => {
                        paste.extend_from_slice(&rest[..at]);
                        pos += at + END.len();
                        out.push(Input::Paste(normalize_paste(&paste)));
                        continue;
                    }
                    None => {
                        // Keep a possible partial end marker in the buffer.
                        let keep = partial_suffix(rest, END);
                        paste.extend_from_slice(&rest[..rest.len() - keep]);
                        pos += rest.len() - keep;
                        self.paste = Some(paste);
                        break;
                    }
                }
            }
            if pos >= self.buf.len() {
                break;
            }
            match self.step(&self.buf[pos..], flush) {
                Step::Done(n, input) => {
                    pos += n.max(1);
                    match input {
                        Some(Input::Paste(_)) => {
                            // The start marker: open a paste.
                            self.paste = Some(Vec::new());
                        }
                        Some(i) => out.push(i),
                        None => {}
                    }
                }
                Step::Incomplete => break,
            }
        }
        self.buf.drain(..pos);
        out
    }

    fn step(&self, b: &[u8], flush: bool) -> Step {
        let first = b[0];
        if first != ESC {
            return match decode_char(b) {
                CharStep::Char(c, n) => Step::Done(n, Some(Input::Key(key_of_char(c)))),
                CharStep::Incomplete if !flush => Step::Incomplete,
                // A truncated or invalid UTF-8 byte is dropped.
                _ => Step::Done(1, None),
            };
        }
        if b.len() == 1 {
            return if flush {
                Step::Done(1, Some(Input::Key(Key::named("esc"))))
            } else {
                Step::Incomplete
            };
        }
        match b[1] {
            b'[' => match self.csi(b, 2) {
                Step::Incomplete if flush => Step::Done(2, Some(Input::Key(alt_char('[')))),
                s => s,
            },
            b'O' => {
                if b.len() < 3 {
                    return if flush {
                        Step::Done(2, Some(Input::Key(alt_char('O'))))
                    } else {
                        Step::Incomplete
                    };
                }
                match ss3_key(b[2]) {
                    Some(k) => Step::Done(3, Some(Input::Key(k))),
                    None => Step::Done(2, Some(Input::Key(alt_char('O')))),
                }
            }
            b']' | b'P' | b'_' | b'^' | b'X' => match string_end(b) {
                StrEnd::End(n) => Step::Done(
                    n,
                    Some(Input::Reply(String::from_utf8_lossy(&b[..n]).into_owned())),
                ),
                StrEnd::Broken => Step::Done(2, Some(Input::Key(alt_char(b[1] as char)))),
                StrEnd::Incomplete if flush => {
                    Step::Done(2, Some(Input::Key(alt_char(b[1] as char))))
                }
                StrEnd::Incomplete => Step::Incomplete,
            },
            ESC => {
                // ESC ESC: alt + an escape sequence (`ESC ESC [ A` is
                // alt+up in several terminals), or alt+Escape.
                if b.len() == 2 {
                    return if flush {
                        Step::Done(2, Some(Input::Key(Key::named("esc").with_alt())))
                    } else {
                        Step::Incomplete
                    };
                }
                match self.step(&b[1..], flush) {
                    Step::Done(n, Some(Input::Key(k))) if b[2] == b'[' || b[2] == b'O' => {
                        Step::Done(n + 1, Some(Input::Key(k.with_alt())))
                    }
                    Step::Incomplete => Step::Incomplete,
                    _ => Step::Done(2, Some(Input::Key(Key::named("esc").with_alt()))),
                }
            }
            _ => match decode_char(&b[1..]) {
                CharStep::Char(c, n) => {
                    Step::Done(n + 1, Some(Input::Key(key_of_char(c).with_alt())))
                }
                CharStep::Incomplete if !flush => Step::Incomplete,
                _ => Step::Done(1, Some(Input::Key(Key::named("esc")))),
            },
        }
    }

    /// A CSI sequence starting at `b[start]` (just after `ESC [`).
    fn csi(&self, b: &[u8], start: usize) -> Step {
        // X10 mouse: `ESC [ M` then three raw bytes.
        if b.len() > start && b[start] == b'M' {
            if b.len() < start + 4 {
                return Step::Incomplete;
            }
            let cb = b[start + 1].wrapping_sub(32) as u32;
            let x = (b[start + 2] as u32).saturating_sub(33);
            let y = (b[start + 3] as u32).saturating_sub(33);
            return Step::Done(start + 4, Some(Input::Mouse(mouse_of(cb, x, y, None))));
        }
        let mut i = start;
        while i < b.len() && (0x30..=0x3f).contains(&b[i]) {
            i += 1;
        }
        let params_end = i;
        while i < b.len() && (0x20..=0x2f).contains(&b[i]) {
            i += 1;
        }
        if i >= b.len() {
            return if i - start > 256 {
                Step::Done(i, None)
            } else {
                Step::Incomplete
            };
        }
        let fin = b[i];
        if !(0x40..=0x7e).contains(&fin) {
            // Not a sequence after all: alt+[ and the rest as keys.
            return Step::Done(2, Some(Input::Key(alt_char('['))));
        }
        let n = i + 1;
        let params = &b[start..params_end];
        let inter = &b[params_end..i];
        let whole = || Input::Reply(String::from_utf8_lossy(&b[..n]).into_owned());

        // Linux console F1–F5: `ESC [ [ A` … `ESC [ [ E`.
        if fin == b'[' && params.is_empty() && inter.is_empty() {
            if b.len() < n + 1 {
                return Step::Incomplete;
            }
            let k = match b[n] {
                b'A' => "f1",
                b'B' => "f2",
                b'C' => "f3",
                b'D' => "f4",
                b'E' => "f5",
                _ => return Step::Done(n + 1, None),
            };
            return Step::Done(n + 1, Some(Input::Key(Key::named(k))));
        }

        let prefix = params.first().copied().filter(|c| b"<=>?".contains(c));
        if prefix == Some(b'<') && (fin == b'M' || fin == b'm') {
            let nums = numbers(&params[1..]);
            let (cb, x, y) = (
                nums.first().copied().unwrap_or(0),
                nums.get(1).copied().unwrap_or(1).saturating_sub(1),
                nums.get(2).copied().unwrap_or(1).saturating_sub(1),
            );
            return Step::Done(n, Some(Input::Mouse(mouse_of(cb, x, y, Some(fin == b'M')))));
        }
        if prefix.is_some() || !inter.is_empty() {
            return Step::Done(n, Some(whole()));
        }
        let groups = groups(params);
        let p = |k: usize| groups.get(k).and_then(|g| g.first()).copied();
        let mods = mods_of(p(1).unwrap_or(1));
        let key = |name: &str| Step::Done(n, Some(Input::Key(Key::named(name).with_mods(mods))));
        match fin {
            b'A' => key("up"),
            b'B' => key("down"),
            b'C' => key("right"),
            b'D' => key("left"),
            b'H' => key("home"),
            b'F' => key("end"),
            b'E' => key("begin"),
            b'P' => key("f1"),
            b'Q' => key("f2"),
            b'S' => key("f4"),
            b'R' => {
                // F3 with modifiers is `ESC [ 1 ; m R`; a cursor position
                // report is `ESC [ row ; col R`.
                if self.expect_cpr || (p(0).unwrap_or(1) != 1 && groups.len() == 2) {
                    Step::Done(n, Some(whole()))
                } else {
                    key("f3")
                }
            }
            b'Z' => Step::Done(
                n,
                Some(Input::Key(Key::named("tab").with_mods(Mods {
                    shift: true,
                    ..mods
                }))),
            ),
            b'I' if params.is_empty() => Step::Done(n, Some(Input::Focus(true))),
            b'O' if params.is_empty() => Step::Done(n, Some(Input::Focus(false))),
            b'~' => {
                let code = p(0).unwrap_or(0);
                if code == 200 {
                    return Step::Done(n, Some(Input::Paste(String::new())));
                }
                if code == 201 {
                    return Step::Done(n, None);
                }
                if code == 27 {
                    // xterm modifyOtherKeys: ESC [ 27 ; mods ; code ~
                    let Some(c) = p(2).and_then(char::from_u32) else {
                        return Step::Done(n, None);
                    };
                    return Step::Done(n, Some(Input::Key(kitty_key(c as u32).with_mods(mods))));
                }
                match tilde_key(code) {
                    Some(name) => key(name),
                    None => Step::Done(n, None),
                }
            }
            b'u' => {
                let code = p(0).unwrap_or(0);
                let event = groups.get(1).and_then(|g| g.get(1)).copied().unwrap_or(1);
                if event == 3 {
                    return Step::Done(n, None); // a release
                }
                let mut k = kitty_key(code).with_mods(mods);
                // The text-as-code-points field (flag 16), when present.
                if let Some(text) = groups.get(2) {
                    let t: String = text.iter().filter_map(|c| char::from_u32(*c)).collect();
                    if !t.is_empty() && !k.ctrl && !k.alt && !k.super_ {
                        k.text = t;
                    }
                }
                Step::Done(n, Some(Input::Key(k)))
            }
            b't' | b'n' | b'c' | b'y' => Step::Done(n, Some(whole())),
            _ => Step::Done(n, None),
        }
    }
}

enum CharStep {
    Char(char, usize),
    Incomplete,
    Invalid,
}

fn decode_char(b: &[u8]) -> CharStep {
    let first = b[0];
    let need = match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return CharStep::Invalid,
    };
    if b.len() < need {
        // Only incomplete if every byte present is a continuation byte.
        if b[1..].iter().all(|c| (0x80..=0xbf).contains(c)) {
            return CharStep::Incomplete;
        }
        return CharStep::Invalid;
    }
    match std::str::from_utf8(&b[..need]) {
        Ok(s) => CharStep::Char(s.chars().next().expect("non-empty"), need),
        Err(_) => CharStep::Invalid,
    }
}

/// The key a single character (or control byte) is.
fn key_of_char(c: char) -> Key {
    match c {
        '\r' => Key::named("enter"),
        '\t' => Key::named("tab"),
        '\x7f' => Key::named("backspace"),
        '\x1b' => Key::named("esc"),
        '\0' => Key::named("space").with_mods(Mods {
            ctrl: true,
            ..Mods::default()
        }),
        '\x01'..='\x1a' => {
            let letter = (b'a' + (c as u8 - 1)) as char;
            Key::char(letter).with_mods(Mods {
                ctrl: true,
                ..Mods::default()
            })
        }
        '\x1c'..='\x1f' => {
            let sym = ['\\', ']', '^', '_'][(c as u8 - 0x1c) as usize];
            Key::char(sym).with_mods(Mods {
                ctrl: true,
                ..Mods::default()
            })
        }
        c if (c as u32) < 0x20 || ('\u{80}'..='\u{9f}').contains(&c) => Key::named("unknown"),
        c => Key::char(c),
    }
}

fn alt_char(c: char) -> Key {
    key_of_char(c).with_alt()
}

fn ss3_key(b: u8) -> Option<Key> {
    Some(Key::named(match b {
        b'A' => "up",
        b'B' => "down",
        b'C' => "right",
        b'D' => "left",
        b'H' => "home",
        b'F' => "end",
        b'E' => "begin",
        b'P' => "f1",
        b'Q' => "f2",
        b'R' => "f3",
        b'S' => "f4",
        b'M' => "enter",
        _ => return None,
    }))
}

fn tilde_key(code: u32) -> Option<&'static str> {
    Some(match code {
        1 | 7 => "home",
        2 => "insert",
        3 => "delete",
        4 | 8 => "end",
        5 => "pgup",
        6 => "pgdown",
        11 => "f1",
        12 => "f2",
        13 => "f3",
        14 => "f4",
        15 => "f5",
        17 => "f6",
        18 => "f7",
        19 => "f8",
        20 => "f9",
        21 => "f10",
        23 => "f11",
        24 => "f12",
        25 => "f13",
        26 => "f14",
        28 => "f15",
        29 => "f16",
        31 => "f17",
        32 => "f18",
        33 => "f19",
        34 => "f20",
        57427 => "begin",
        _ => return None,
    })
}

/// A kitty-protocol key code: a Unicode code point, or one of the
/// protocol's private-use functional keys.
fn kitty_key(code: u32) -> Key {
    let name: &str = match code {
        9 => "tab",
        13 => "enter",
        27 => "esc",
        8 | 127 => "backspace",
        57358 => "caps_lock",
        57359 => "scroll_lock",
        57360 => "num_lock",
        57361 => "print_screen",
        57362 => "pause",
        57363 => "menu",
        57376..=57398 => {
            return Key::named(&format!("f{}", code - 57376 + 13));
        }
        57399..=57408 => {
            return Key::char(char::from(b'0' + (code - 57399) as u8));
        }
        57409 => return Key::char('.'),
        57410 => return Key::char('/'),
        57411 => return Key::char('*'),
        57412 => return Key::char('-'),
        57413 => return Key::char('+'),
        57414 => "enter",
        57415 => return Key::char('='),
        57416 => return Key::char(','),
        57417 => "left",
        57418 => "right",
        57419 => "up",
        57420 => "down",
        57421 => "pgup",
        57422 => "pgdown",
        57423 => "home",
        57424 => "end",
        57425 => "insert",
        57426 => "delete",
        57427 => "begin",
        57428..=57440 => "media",
        57441 => "left_shift",
        57442 => "left_ctrl",
        57443 => "left_alt",
        57444 => "left_super",
        57445 => "left_hyper",
        57446 => "left_meta",
        57447 => "right_shift",
        57448 => "right_ctrl",
        57449 => "right_alt",
        57450 => "right_super",
        57451 => "right_hyper",
        57452 => "right_meta",
        57453 => "iso_level3_shift",
        57454 => "iso_level5_shift",
        c => {
            return match char::from_u32(c) {
                Some(ch) if !ch.is_control() => Key::char(ch),
                Some(ch) => key_of_char(ch),
                None => Key::named("unknown"),
            };
        }
    };
    Key::named(name)
}

fn mouse_of(cb: u32, x: u32, y: u32, press: Option<bool>) -> Mouse {
    let low = cb & 3;
    let motion = cb & 32 != 0;
    let wheel = cb & 64 != 0;
    let extra = cb & 128 != 0;
    let (action, button) = if wheel {
        ("wheel", ["up", "down", "left", "right"][low as usize])
    } else if extra {
        let b = ["b8", "b9", "b10", "b11"][low as usize];
        match press {
            Some(false) => ("up", b),
            _ if motion => ("drag", b),
            _ => ("down", b),
        }
    } else {
        let b = ["left", "middle", "right", "none"][low as usize];
        if motion {
            if low == 3 {
                ("move", "none")
            } else {
                ("drag", b)
            }
        } else {
            match press {
                Some(true) => ("down", b),
                Some(false) => ("up", b),
                // X10: a release is button 3, unattributed.
                None if low == 3 => ("up", "none"),
                None => ("down", b),
            }
        }
    };
    Mouse {
        action,
        button,
        x,
        y,
        shift: cb & 4 != 0,
        alt: cb & 8 != 0,
        ctrl: cb & 16 != 0,
    }
}

/// `;`-separated numbers (sub-parameters after `:` ignored).
fn numbers(params: &[u8]) -> Vec<u32> {
    groups(params)
        .into_iter()
        .map(|g| g.first().copied().unwrap_or(0))
        .collect()
}

/// `;`-separated groups of `:`-separated numbers; an empty field is 0.
fn groups(params: &[u8]) -> Vec<Vec<u32>> {
    if params.is_empty() {
        return Vec::new();
    }
    params
        .split(|c| *c == b';')
        .map(|g| {
            g.split(|c| *c == b':')
                .map(|n| {
                    n.iter().filter(|c| c.is_ascii_digit()).fold(0u32, |a, c| {
                        a.saturating_mul(10).saturating_add((c - b'0') as u32)
                    })
                })
                .collect()
        })
        .collect()
}

enum StrEnd {
    End(usize),
    Incomplete,
    Broken,
}

/// The end of an OSC/DCS/APC/PM/SOS string starting at `b[0]` (ESC):
/// BEL or ST (`ESC \`). A control byte inside means it was never a
/// string (alt+`]` followed by typing).
fn string_end(b: &[u8]) -> StrEnd {
    let mut i = 2;
    while i < b.len() {
        match b[i] {
            0x07 => return StrEnd::End(i + 1),
            ESC => {
                if i + 1 >= b.len() {
                    return StrEnd::Incomplete;
                }
                return if b[i + 1] == b'\\' {
                    StrEnd::End(i + 2)
                } else {
                    StrEnd::Broken
                };
            }
            c if c < 0x20 => return StrEnd::Broken,
            _ => {}
        }
        i += 1;
        if i > 64 * 1024 {
            return StrEnd::Broken;
        }
    }
    StrEnd::Incomplete
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The length of the longest suffix of `hay` that is a proper prefix of
/// `needle` (a marker split across reads).
fn partial_suffix(hay: &[u8], needle: &[u8]) -> usize {
    (1..needle.len())
        .rev()
        .find(|&k| k <= hay.len() && hay[hay.len() - k..] == needle[..k])
        .unwrap_or(0)
}

fn normalize_paste(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.replace("\r\n", "\n").replace('\r', "\n")
}
