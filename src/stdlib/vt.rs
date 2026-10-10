//! `vt` — a terminal's screen: the bytes a program on a terminal writes
//! (an xterm's escape sequences) read into a grid of cells with a
//! scrollback, and that grid drawn as a canvas's operations.
//!
//!   let t = vt.new(24, 80)
//!   let reply = vt.feed(t, "\u{1b}[1;31mred\u{1b}[0m\r\n")   // what the terminal answers (a cursor report), often ""
//!   vt.info(t).cursor                     // (line, col): the line counted from the first ever kept
//!   vt.text(t, l0, c0, l1, c1)            // a selection's text
//!   vt.render(t, #{ "cell_w": 7.8, "cell_h": 17, "size": 13, … })  // canvas operations
//!
//! What it reads: UTF-8 (wide characters take two cells, combining marks
//! join the cell before), C0 controls, ESC (save and restore the cursor,
//! index, reverse index, next line, reset, keypad modes, character sets
//! with DEC's line drawing), CSI (cursor movement, erase in display and
//! line, insert and delete characters and lines, scroll up and down,
//! scroll regions, tab stops, repeat, SGR with 16, 256 and true colours,
//! bold, dim, italic, underline, inverse, hidden, strikethrough; modes:
//! insert, newline, application cursor keys, origin, autowrap, the cursor
//! shown, the alternate screen (47, 1047, 1049), bracketed paste (2004),
//! mouse reporting (1000, 1002, 1003; 1006's encoding), focus events
//! (1004); the cursor's style; device status, attributes and a mode's
//! state answered), OSC (0 and 2 a title, 7 the working directory, 8
//! hyperlinks, 10 and 11 the colours asked), and DCS, SOS, PM and APC
//! read and ignored. Lines are numbered from the first line ever kept, so
//! a selection stays on its text while the scrollback (10,000 lines by
//! default) moves; a resize rewraps the main screen's lines.
//!
//! Screens are `Screen { id }` handles into a registry (as `proc`'s are);
//! `vt.free` forgets one. A screen may be fed on one thread and drawn on
//! another (a terminal's reader task feeds it; the window draws it).

use crate::ast::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use unicode_width::UnicodeWidthChar;

type Res = Result<Value, Box<dyn std::error::Error>>;

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}
fn int(i: i64) -> Value {
    Value::Integer(i)
}
fn fl(f: f32) -> Value {
    Value::Float(f as f64)
}
fn obj(type_name: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Struct {
        type_name: type_name.to_string(),
        fields: Arc::new(m),
    }
}
#[allow(dead_code)]
fn vmap(fields: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}
fn tuple(vs: Vec<Value>) -> Value {
    Value::Tuple(Arc::new(vs))
}

pub fn create_vt_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("new", 3),
        ("feed", 2),
        ("resize", 3),
        ("info", 1),
        ("claim", 1),
        ("render", 2),
        ("text", 5),
        ("line", 2),
        ("word_at", 3),
        ("link_at", 3),
        ("find", 3),
        ("clear", 1),
        ("reset", 1),
        ("set_colors", 2),
        ("free", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("vt.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

// ── the screen ───────────────────────────────────────────────────────

const BOLD: u16 = 1;
const DIM: u16 = 2;
const ITALIC: u16 = 4;
const UNDERLINE: u16 = 8;
const BLINK: u16 = 16;
const INVERSE: u16 = 32;
const HIDDEN: u16 = 64;
const STRIKE: u16 = 128;

/// A colour: 0 the default, `IDX | n` the palette's `n`, `RGB | rgb`.
const IDX: u32 = 0x100_0000;
const RGB: u32 = 0x200_0000;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
struct Attr {
    fg: u32,
    bg: u32,
    fl: u16,
    /// A hyperlink (OSC 8): its index + 1 in `links`, 0 for none.
    link: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Cell {
    ch: char,
    /// 1, 2 (a wide character's first half) or 0 (its second half).
    w: u8,
    a: Attr,
}

impl Cell {
    fn blank(a: Attr) -> Cell {
        Cell {
            ch: ' ',
            w: 1,
            a: Attr { fg: a.fg, bg: a.bg, fl: 0, link: 0 },
        }
    }
    fn is_blank(&self) -> bool {
        self.ch == ' ' && self.a.bg == 0 && self.a.fl & (INVERSE | UNDERLINE | STRIKE) == 0 && self.w == 1
    }
}

const BLANK: Cell = Cell {
    ch: ' ',
    w: 1,
    a: Attr { fg: 0, bg: 0, fl: 0, link: 0 },
};

#[derive(Clone, Debug, Default)]
struct Line {
    cells: Vec<Cell>,
    /// It goes on in the next line (it was wrapped there).
    wrapped: bool,
    /// Combining marks after a cell's character, by column.
    extra: Option<Box<Vec<(u16, String)>>>,
}

impl Line {
    fn new(cols: usize, a: Attr) -> Line {
        Line {
            cells: vec![Cell::blank(a); cols],
            wrapped: false,
            extra: None,
        }
    }
    fn fit(&mut self, cols: usize) {
        if self.cells.len() < cols {
            self.cells.resize(cols, BLANK);
        }
    }
    fn trimmed_len(&self) -> usize {
        let mut n = self.cells.len();
        while n > 0 && self.cells[n - 1].is_blank() {
            n -= 1;
        }
        n
    }
    fn is_blank(&self) -> bool {
        self.trimmed_len() == 0
    }
    fn extra_at(&self, col: usize) -> Option<&str> {
        self.extra.as_ref()?.iter().find(|(c, _)| *c as usize == col).map(|(_, t)| t.as_str())
    }
    fn clear_extra_from(&mut self, from: usize, to: usize) {
        if let Some(e) = self.extra.as_mut() {
            e.retain(|(c, _)| (*c as usize) < from || (*c as usize) >= to);
            if e.is_empty() {
                self.extra = None;
            }
        }
    }
    /// The text of cells `from..to` (a wide character once).
    fn text(&self, from: usize, to: usize) -> String {
        let mut out = String::new();
        for c in from..to.min(self.cells.len()) {
            let cell = &self.cells[c];
            if cell.w == 0 {
                continue;
            }
            out.push(cell.ch);
            if let Some(x) = self.extra_at(c) {
                out.push_str(x);
            }
        }
        out
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Saved {
    row: usize,
    col: usize,
    a: Attr,
    origin: bool,
    g0: u8,
    g1: u8,
    gl: u8,
    pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Ground,
    Esc,
    EscInter,
    Csi,
    CsiIgnore,
    Osc,
    OscEsc,
    Str,
    StrEsc,
}

struct Term {
    rows: usize,
    cols: usize,
    /// The main screen and its scrollback: the last `rows` lines are the screen.
    lines: VecDeque<Line>,
    /// Lines that left the scrollback's top (numbering stays put).
    evicted: i64,
    max_scrollback: usize,
    /// The alternate screen, while it is shown.
    alt: Option<Vec<Line>>,
    row: usize,
    col: usize,
    pending: bool,
    a: Attr,
    origin: bool,
    autowrap: bool,
    insert: bool,
    newline: bool,
    app_cursor: bool,
    app_keypad: bool,
    cursor_visible: bool,
    cursor_style: u8,
    bracketed: bool,
    mouse: u16,
    mouse_sgr: bool,
    focus_events: bool,
    reverse: bool,
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    g0: u8,
    g1: u8,
    gl: u8,
    saved_main: Saved,
    saved_alt: Saved,
    title: String,
    cwd: String,
    bell: i64,
    rev: i64,
    notified: bool,
    links: Vec<String>,
    link: u16,
    reply: String,
    last: char,
    fg_rgb: (u8, u8, u8),
    bg_rgb: (u8, u8, u8),
    // the parser
    st: St,
    params: Vec<u32>,
    /// Which params were joined to the one before by a colon.
    sub: Vec<bool>,
    cur: u32,
    has_cur: bool,
    colon_next: bool,
    private: u8,
    inter: Vec<u8>,
    osc: Vec<u8>,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
}

impl Term {
    fn new(rows: usize, cols: usize, scrollback: usize) -> Term {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let mut lines = VecDeque::new();
        for _ in 0..rows {
            lines.push_back(Line::new(cols, Attr::default()));
        }
        Term {
            rows,
            cols,
            lines,
            evicted: 0,
            max_scrollback: scrollback,
            alt: None,
            row: 0,
            col: 0,
            pending: false,
            a: Attr::default(),
            origin: false,
            autowrap: true,
            insert: false,
            newline: false,
            app_cursor: false,
            app_keypad: false,
            cursor_visible: true,
            cursor_style: 0,
            bracketed: false,
            mouse: 0,
            mouse_sgr: false,
            focus_events: false,
            reverse: false,
            top: 0,
            bottom: rows - 1,
            tabs: Term::default_tabs(cols),
            g0: b'B',
            g1: b'B',
            gl: 0,
            saved_main: Saved { g0: b'B', g1: b'B', ..Default::default() },
            saved_alt: Saved { g0: b'B', g1: b'B', ..Default::default() },
            title: String::new(),
            cwd: String::new(),
            bell: 0,
            rev: 0,
            notified: false,
            links: Vec::new(),
            link: 0,
            reply: String::new(),
            last: ' ',
            fg_rgb: (0xd4, 0xd4, 0xd8),
            bg_rgb: (0x18, 0x18, 0x1b),
            st: St::Ground,
            params: Vec::new(),
            sub: Vec::new(),
            cur: 0,
            has_cur: false,
            colon_next: false,
            private: 0,
            inter: Vec::new(),
            osc: Vec::new(),
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
        }
    }

    fn default_tabs(cols: usize) -> Vec<bool> {
        (0..cols).map(|c| c % 8 == 0 && c > 0).collect()
    }

    // ── lines ──

    fn screen_base(&self) -> usize {
        self.lines.len() - self.rows
    }

    fn line_mut(&mut self, r: usize) -> &mut Line {
        if let Some(alt) = self.alt.as_mut() {
            return &mut alt[r];
        }
        let b = self.lines.len() - self.rows;
        &mut self.lines[b + r]
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn line(&self, r: usize) -> &Line {
        if let Some(alt) = self.alt.as_ref() {
            return &alt[r];
        }
        &self.lines[self.lines.len() - self.rows + r]
    }

    /// The first line's number (the scrollback's top).
    fn first_id(&self) -> i64 {
        self.evicted
    }
    /// The screen's first line's number.
    fn screen_id(&self) -> i64 {
        self.evicted + (self.lines.len() - self.rows) as i64
    }

    /// The line numbered `id` as drawn now (the alternate screen's own
    /// rows stand at the screen's numbers).
    fn line_by_id(&self, id: i64) -> Option<&Line> {
        if let Some(alt) = self.alt.as_ref() {
            let r = id - self.screen_id();
            return if r >= 0 && (r as usize) < self.rows { Some(&alt[r as usize]) } else { None };
        }
        let i = id - self.evicted;
        if i < 0 || i as usize >= self.lines.len() {
            None
        } else {
            Some(&self.lines[i as usize])
        }
    }

    fn last_id(&self) -> i64 {
        self.screen_id() + self.rows as i64 - 1
    }

    fn blank_line(&self) -> Line {
        Line::new(self.cols, self.a)
    }

    /// Lines `top..=bottom` up by `n`: the top ones gone (into the
    /// scrollback when the region is the whole main screen).
    fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.bottom + 1 - self.top).max(1);
        if self.alt.is_none() && self.top == 0 && self.bottom == self.rows - 1 {
            for _ in 0..n {
                let base = self.screen_base();
                // the screen's top line becomes the scrollback's last: kept short
                let l = &mut self.lines[base];
                let keep = l.trimmed_len();
                if !l.wrapped {
                    l.cells.truncate(keep);
                }
                let bl = self.blank_line();
                self.lines.push_back(bl);
            }
            while self.lines.len() - self.rows > self.max_scrollback {
                self.lines.pop_front();
                self.evicted += 1;
            }
            return;
        }
        for _ in 0..n {
            let bl = self.blank_line();
            let (top, bottom) = (self.top, self.bottom);
            if let Some(alt) = self.alt.as_mut() {
                alt.remove(top);
                alt.insert(bottom, bl);
            } else {
                let b = self.screen_base();
                self.lines.remove(b + top);
                self.lines.insert(b + bottom, bl);
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let n = n.min(self.bottom + 1 - self.top).max(1);
        for _ in 0..n {
            let bl = self.blank_line();
            let (top, bottom) = (self.top, self.bottom);
            if let Some(alt) = self.alt.as_mut() {
                alt.remove(bottom);
                alt.insert(top, bl);
            } else {
                let b = self.screen_base();
                self.lines.remove(b + bottom);
                self.lines.insert(b + top, bl);
            }
        }
    }

    fn linefeed(&mut self) {
        self.pending = false;
        if self.row == self.bottom {
            self.scroll_up(1);
        } else if self.row < self.rows - 1 {
            self.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.pending = false;
        if self.row == self.top {
            self.scroll_down(1);
        } else if self.row > 0 {
            self.row -= 1;
        }
    }

    // ── characters ──

    fn map_charset(&self, c: char) -> char {
        let set = if self.gl == 0 { self.g0 } else { self.g1 };
        if set != b'0' || !('\u{5f}'..='\u{7e}').contains(&c) {
            return c;
        }
        const DEC: [char; 32] = [
            ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─', '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
        ];
        DEC[(c as usize) - 0x5f]
    }

    fn put(&mut self, c0: char) {
        let c = self.map_charset(c0);
        let w = match c.width() {
            Some(w) => w,
            None => 0,
        };
        if w == 0 {
            // a combining mark (or a zero-width joiner): the cell before's
            let col = if self.pending { self.col } else { self.col.saturating_sub(1) };
            let row = self.row;
            let l = self.line_mut(row);
            let col = if col < l.cells.len() && l.cells[col].w == 0 && col > 0 { col - 1 } else { col };
            let e = l.extra.get_or_insert_with(|| Box::new(Vec::new()));
            if let Some(x) = e.iter_mut().find(|(cc, _)| *cc as usize == col) {
                x.1.push(c);
            } else {
                e.push((col as u16, c.to_string()));
            }
            return;
        }
        let w = w.min(2);
        if self.pending && self.autowrap {
            let row = self.row;
            self.line_mut(row).wrapped = true;
            self.col = 0;
            self.linefeed();
        }
        self.pending = false;
        if w == 2 && self.col + 1 >= self.cols {
            if self.autowrap && self.cols > 1 {
                let (row, col) = (self.row, self.col);
                let a = self.a;
                let l = self.line_mut(row);
                if col < l.cells.len() {
                    l.cells[col] = Cell::blank(a);
                }
                l.wrapped = true;
                self.col = 0;
                self.linefeed();
            } else {
                self.col = self.cols.saturating_sub(2);
            }
        }
        let (row, col, cols, insert) = (self.row, self.col, self.cols, self.insert);
        let mut a = self.a;
        a.link = self.link;
        let l = self.line_mut(row);
        l.fit(cols);
        if insert {
            for _ in 0..w {
                l.cells.insert(col, BLANK);
            }
            l.cells.truncate(cols);
        }
        // a wide character half overwritten: its other half blanked
        if l.cells[col].w == 0 && col > 0 {
            l.cells[col - 1] = Cell::blank(l.cells[col - 1].a);
        }
        let end = col + w;
        if end < cols && l.cells[end].w == 0 {
            l.cells[end] = Cell::blank(l.cells[end].a);
        }
        if l.extra.is_some() {
            l.clear_extra_from(col, end);
        }
        l.cells[col] = Cell { ch: c, w: w as u8, a };
        if w == 2 && col + 1 < cols {
            l.cells[col + 1] = Cell { ch: ' ', w: 0, a };
        }
        self.last = c0;
        if end >= cols {
            self.col = cols - 1;
            self.pending = self.autowrap;
        } else {
            self.col = end;
        }
    }

    // ── erasing ──

    fn erase_cells(&mut self, row: usize, from: usize, to: usize) {
        let a = self.a;
        let cols = self.cols;
        let l = self.line_mut(row);
        l.fit(cols);
        let to = to.min(l.cells.len());
        // a wide character cut in two: both halves go
        let from = if from > 0 && from < l.cells.len() && l.cells[from].w == 0 { from - 1 } else { from };
        let to = if to < l.cells.len() && l.cells[to].w == 0 { to + 1 } else { to };
        for c in from..to.min(l.cells.len()) {
            l.cells[c] = Cell::blank(a);
        }
        l.clear_extra_from(from, to);
        if to >= cols {
            l.wrapped = false;
        }
    }

    fn erase_display(&mut self, how: u32) {
        match how {
            0 => {
                let (r, c, rows, cols) = (self.row, self.col, self.rows, self.cols);
                self.erase_cells(r, c, cols);
                for rr in r + 1..rows {
                    self.erase_cells(rr, 0, cols);
                }
            }
            1 => {
                let (r, c, cols) = (self.row, self.col, self.cols);
                for rr in 0..r {
                    self.erase_cells(rr, 0, cols);
                }
                self.erase_cells(r, 0, c + 1);
            }
            2 => {
                let (rows, cols) = (self.rows, self.cols);
                for rr in 0..rows {
                    self.erase_cells(rr, 0, cols);
                }
            }
            3 => {
                if self.alt.is_none() {
                    let n = self.screen_base();
                    for _ in 0..n {
                        self.lines.pop_front();
                    }
                    self.evicted += n as i64;
                }
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, how: u32) {
        let (r, c, cols) = (self.row, self.col, self.cols);
        match how {
            0 => self.erase_cells(r, c, cols),
            1 => self.erase_cells(r, 0, c + 1),
            2 => self.erase_cells(r, 0, cols),
            _ => {}
        }
    }

    fn insert_chars(&mut self, n: usize) {
        let (r, c, cols) = (self.row, self.col, self.cols);
        let a = self.a;
        let l = self.line_mut(r);
        l.fit(cols);
        for _ in 0..n.min(cols - c) {
            l.cells.insert(c, Cell::blank(a));
        }
        l.cells.truncate(cols);
        if l.cells[cols - 1].w == 2 {
            l.cells[cols - 1] = Cell::blank(a);
        }
        l.extra = None;
    }

    fn delete_chars(&mut self, n: usize) {
        let (r, c, cols) = (self.row, self.col, self.cols);
        let a = self.a;
        let l = self.line_mut(r);
        l.fit(cols);
        let n = n.min(cols - c);
        for _ in 0..n {
            l.cells.remove(c);
            l.cells.push(Cell::blank(a));
        }
        if c < cols && l.cells[c].w == 0 {
            l.cells[c] = Cell::blank(a);
        }
        l.extra = None;
    }

    fn insert_lines(&mut self, n: usize) {
        if self.row < self.top || self.row > self.bottom {
            return;
        }
        let saved = self.top;
        self.top = self.row;
        self.scroll_down(n);
        self.top = saved;
        self.col = 0;
        self.pending = false;
    }

    fn delete_lines(&mut self, n: usize) {
        if self.row < self.top || self.row > self.bottom {
            return;
        }
        let saved = self.top;
        self.top = self.row;
        // within the region: never into the scrollback
        let full = self.top == 0 && self.bottom == self.rows - 1 && self.alt.is_none();
        if full {
            for _ in 0..n.min(self.rows) {
                let bl = self.blank_line();
                let b = self.screen_base();
                self.lines.remove(b);
                self.lines.insert(b + self.rows - 1, bl);
            }
        } else {
            self.scroll_up(n);
        }
        self.top = saved;
        self.col = 0;
        self.pending = false;
    }

    // ── the cursor ──

    fn goto(&mut self, row: i64, col: i64) {
        let (lo, hi) = if self.origin { (self.top as i64, self.bottom as i64) } else { (0, self.rows as i64 - 1) };
        let r = if self.origin { row + self.top as i64 } else { row };
        self.row = r.clamp(lo, hi) as usize;
        self.col = col.clamp(0, self.cols as i64 - 1) as usize;
        self.pending = false;
    }

    fn save_cursor(&mut self) {
        let s = Saved {
            row: self.row,
            col: self.col,
            a: self.a,
            origin: self.origin,
            g0: self.g0,
            g1: self.g1,
            gl: self.gl,
            pending: self.pending,
        };
        if self.alt.is_some() {
            self.saved_alt = s;
        } else {
            self.saved_main = s;
        }
    }

    fn restore_cursor(&mut self) {
        let s = if self.alt.is_some() { self.saved_alt } else { self.saved_main };
        self.row = s.row.min(self.rows - 1);
        self.col = s.col.min(self.cols - 1);
        self.a = s.a;
        self.origin = s.origin;
        self.g0 = s.g0;
        self.g1 = s.g1;
        self.gl = s.gl;
        self.pending = s.pending;
    }

    fn enter_alt(&mut self, clear: bool) {
        if self.alt.is_none() {
            self.alt = Some((0..self.rows).map(|_| Line::new(self.cols, Attr::default())).collect());
        } else if clear {
            let cols = self.cols;
            for l in self.alt.as_mut().unwrap().iter_mut() {
                *l = Line::new(cols, Attr::default());
            }
        }
    }

    fn leave_alt(&mut self) {
        self.alt = None;
        self.row = self.row.min(self.rows - 1);
    }

    fn soft_reset(&mut self) {
        self.cursor_visible = true;
        self.origin = false;
        self.autowrap = true;
        self.insert = false;
        self.app_cursor = false;
        self.app_keypad = false;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.a = Attr::default();
        self.g0 = b'B';
        self.g1 = b'B';
        self.gl = 0;
        self.pending = false;
    }

    fn full_reset(&mut self) {
        let (rows, cols, sb) = (self.rows, self.cols, self.max_scrollback);
        let (fg, bg, ev) = (self.fg_rgb, self.bg_rgb, self.evicted + self.lines.len() as i64);
        *self = Term::new(rows, cols, sb);
        self.fg_rgb = fg;
        self.bg_rgb = bg;
        self.evicted = ev;
    }

    // ── modes ──

    fn set_mode(&mut self, on: bool) {
        let params = std::mem::take(&mut self.params);
        for &p in &params {
            if self.private == b'?' {
                match p {
                    1 => self.app_cursor = on,
                    5 => self.reverse = on,
                    6 => {
                        self.origin = on;
                        self.goto(0, 0);
                    }
                    7 => self.autowrap = on,
                    12 => {}
                    25 => self.cursor_visible = on,
                    47 | 1047 => {
                        if on {
                            self.enter_alt(p == 1047);
                        } else {
                            if p == 1047 && self.alt.is_some() {
                                self.enter_alt(true);
                            }
                            self.leave_alt();
                        }
                    }
                    1048 => {
                        if on {
                            self.save_cursor()
                        } else {
                            self.restore_cursor()
                        }
                    }
                    1049 => {
                        if on {
                            if self.alt.is_none() {
                                self.save_cursor();
                                self.enter_alt(true);
                                self.saved_alt = self.saved_main;
                            }
                        } else if self.alt.is_some() {
                            self.leave_alt();
                            self.restore_cursor();
                        }
                    }
                    9 | 1000 | 1002 | 1003 => self.mouse = if on { p as u16 } else { 0 },
                    1004 => self.focus_events = on,
                    1006 => self.mouse_sgr = on,
                    2004 => self.bracketed = on,
                    _ => {}
                }
            } else {
                match p {
                    4 => self.insert = on,
                    20 => self.newline = on,
                    _ => {}
                }
            }
        }
        self.params = params;
    }

    fn mode_state(&self, p: u32) -> u8 {
        // 1 set, 2 reset, 0 not recognised
        let b = |v: bool| if v { 1 } else { 2 };
        if self.private == b'?' {
            match p {
                1 => b(self.app_cursor),
                5 => b(self.reverse),
                6 => b(self.origin),
                7 => b(self.autowrap),
                25 => b(self.cursor_visible),
                47 | 1047 | 1049 => b(self.alt.is_some()),
                1000 | 1002 | 1003 => b(self.mouse == p as u16),
                1004 => b(self.focus_events),
                1006 => b(self.mouse_sgr),
                2004 => b(self.bracketed),
                _ => 0,
            }
        } else {
            match p {
                4 => b(self.insert),
                20 => b(self.newline),
                _ => 0,
            }
        }
    }

    // ── SGR ──

    fn sgr(&mut self) {
        let ps = std::mem::take(&mut self.params);
        let sub = std::mem::take(&mut self.sub);
        if ps.is_empty() {
            self.a = Attr { link: self.a.link, ..Attr::default() };
        }
        let mut i = 0;
        while i < ps.len() {
            let p = ps[i];
            // the colon-joined parameters after `p`
            let mut j = i + 1;
            while j < ps.len() && sub.get(j).copied().unwrap_or(false) {
                j += 1;
            }
            let subs = &ps[i + 1..j];
            match p {
                0 => self.a = Attr { link: self.a.link, ..Attr::default() },
                1 => self.a.fl |= BOLD,
                2 => self.a.fl |= DIM,
                3 => self.a.fl |= ITALIC,
                4 => {
                    if subs.first() == Some(&0) {
                        self.a.fl &= !UNDERLINE
                    } else {
                        self.a.fl |= UNDERLINE
                    }
                }
                5 | 6 => self.a.fl |= BLINK,
                7 => self.a.fl |= INVERSE,
                8 => self.a.fl |= HIDDEN,
                9 => self.a.fl |= STRIKE,
                21 => self.a.fl |= UNDERLINE,
                22 => self.a.fl &= !(BOLD | DIM),
                23 => self.a.fl &= !ITALIC,
                24 => self.a.fl &= !UNDERLINE,
                25 => self.a.fl &= !BLINK,
                27 => self.a.fl &= !INVERSE,
                28 => self.a.fl &= !HIDDEN,
                29 => self.a.fl &= !STRIKE,
                30..=37 => self.a.fg = IDX | (p - 30),
                39 => self.a.fg = 0,
                40..=47 => self.a.bg = IDX | (p - 40),
                49 => self.a.bg = 0,
                90..=97 => self.a.fg = IDX | (p - 90 + 8),
                100..=107 => self.a.bg = IDX | (p - 100 + 8),
                38 | 48 | 58 => {
                    // 38;5;n  38;2;r;g;b  38:5:n  38:2::r:g:b  38:2:r:g:b
                    let (c, used) = if !subs.is_empty() {
                        (Term::ext_color(subs, true), 0)
                    } else {
                        let rest = &ps[i + 1..];
                        match rest.first() {
                            Some(5) if rest.len() >= 2 => (Some(IDX | (rest[1] & 0xff)), 2),
                            Some(2) if rest.len() >= 4 => (Some(RGB | ((rest[1] & 0xff) << 16) | ((rest[2] & 0xff) << 8) | (rest[3] & 0xff)), 4),
                            Some(_) => (None, rest.len().min(1)),
                            None => (None, 0),
                        }
                    };
                    if let Some(c) = c {
                        if p == 38 {
                            self.a.fg = c
                        } else if p == 48 {
                            self.a.bg = c
                        }
                    }
                    i += used;
                }
                59 => {}
                _ => {}
            }
            i = j.max(i + 1);
        }
        self.params = ps;
        self.sub = sub;
        self.params.clear();
        self.sub.clear();
    }

    fn ext_color(subs: &[u32], _colon: bool) -> Option<u32> {
        match subs.first() {
            Some(5) => subs.get(1).map(|n| IDX | (n & 0xff)),
            Some(2) => {
                // with a colour space id (38:2::r:g:b → 2,0,r,g,b or 2,r,g,b)
                let v = &subs[1..];
                let (r, g, b) = if v.len() >= 4 { (v[1], v[2], v[3]) } else if v.len() == 3 { (v[0], v[1], v[2]) } else { return None };
                Some(RGB | ((r & 0xff) << 16) | ((g & 0xff) << 8) | (b & 0xff))
            }
            _ => None,
        }
    }

    // ── the parser ──

    fn param(&self, i: usize, default: u32) -> u32 {
        match self.params.get(i) {
            Some(&0) | None => default,
            Some(&p) => p,
        }
    }

    fn feed(&mut self, data: &[u8]) {
        for &b in data {
            self.byte(b);
        }
        self.rev += 1;
    }

    fn byte(&mut self, b: u8) {
        // inside a string (OSC, DCS…), bytes are its own
        match self.st {
            St::Osc => {
                match b {
                    0x07 => self.osc_end(),
                    0x1b => self.st = St::OscEsc,
                    0x18 | 0x1a => self.st = St::Ground,
                    _ => {
                        if self.osc.len() < 1 << 16 {
                            self.osc.push(b)
                        }
                    }
                }
                return;
            }
            St::OscEsc => {
                if b == b'\\' {
                    self.osc_end();
                } else {
                    self.osc_end();
                    self.st = St::Esc;
                    self.byte(b);
                }
                return;
            }
            St::Str => {
                match b {
                    0x1b => self.st = St::StrEsc,
                    0x07 | 0x18 | 0x1a => self.st = St::Ground,
                    _ => {}
                }
                return;
            }
            St::StrEsc => {
                self.st = if b == b'\\' { St::Ground } else { St::Str };
                return;
            }
            _ => {}
        }
        // UTF-8 in the ground state
        if self.utf8_need > 0 {
            if b & 0xc0 == 0x80 {
                self.utf8[self.utf8_len] = b;
                self.utf8_len += 1;
                if self.utf8_len == self.utf8_need {
                    let c = std::str::from_utf8(&self.utf8[..self.utf8_len]).ok().and_then(|t| t.chars().next()).unwrap_or('\u{fffd}');
                    self.utf8_need = 0;
                    self.utf8_len = 0;
                    if self.st == St::Ground {
                        self.put(c);
                    }
                }
                return;
            }
            // a broken sequence
            self.utf8_need = 0;
            self.utf8_len = 0;
            if self.st == St::Ground {
                self.put('\u{fffd}');
            }
        }
        if b >= 0x80 && self.st == St::Ground {
            let need = if b & 0xe0 == 0xc0 {
                2
            } else if b & 0xf0 == 0xe0 {
                3
            } else if b & 0xf8 == 0xf0 {
                4
            } else {
                self.put('\u{fffd}');
                return;
            };
            self.utf8[0] = b;
            self.utf8_len = 1;
            self.utf8_need = need;
            return;
        }
        if b < 0x20 || b == 0x7f {
            self.control(b);
            return;
        }
        match self.st {
            St::Ground => self.put(b as char),
            St::Esc => self.esc(b),
            St::EscInter => {
                if (0x20..=0x2f).contains(&b) {
                    self.inter.push(b);
                } else {
                    self.esc_dispatch(b);
                }
            }
            St::Csi => self.csi(b),
            St::CsiIgnore => {
                if (0x40..=0x7e).contains(&b) {
                    self.st = St::Ground;
                }
            }
            _ => {}
        }
    }

    fn control(&mut self, b: u8) {
        match b {
            0x1b => {
                self.st = St::Esc;
                self.inter.clear();
                return;
            }
            0x18 | 0x1a => {
                self.st = St::Ground;
                return;
            }
            _ => {}
        }
        match b {
            0x07 => self.bell += 1,
            0x08 => {
                if self.pending {
                    self.pending = false;
                } else if self.col > 0 {
                    self.col -= 1;
                }
            }
            0x09 => {
                self.pending = false;
                let mut c = self.col + 1;
                while c < self.cols - 1 && !self.tabs[c] {
                    c += 1;
                }
                self.col = c.min(self.cols - 1);
            }
            0x0a..=0x0c => {
                if self.newline {
                    self.col = 0;
                }
                self.linefeed();
            }
            0x0d => {
                self.col = 0;
                self.pending = false;
            }
            0x0e => self.gl = 1,
            0x0f => self.gl = 0,
            _ => {}
        }
    }

    fn esc(&mut self, b: u8) {
        match b {
            b'[' => {
                self.st = St::Csi;
                self.params.clear();
                self.sub.clear();
                self.cur = 0;
                self.has_cur = false;
                self.private = 0;
                self.inter.clear();
            }
            b']' => {
                self.st = St::Osc;
                self.osc.clear();
            }
            b'P' | b'X' | b'^' | b'_' => self.st = St::Str,
            0x20..=0x2f => {
                self.inter.clear();
                self.inter.push(b);
                self.st = St::EscInter;
            }
            _ => self.esc_dispatch(b),
        }
    }

    fn esc_dispatch(&mut self, b: u8) {
        self.st = St::Ground;
        if let Some(&i) = self.inter.first() {
            match (i, b) {
                (b'(', c) => self.g0 = c,
                (b')', c) => self.g1 = c,
                (b'#', b'8') => {
                    // DECALN: the screen filled with E
                    let (rows, cols) = (self.rows, self.cols);
                    for r in 0..rows {
                        let l = self.line_mut(r);
                        *l = Line::new(cols, Attr::default());
                        for c in l.cells.iter_mut() {
                            c.ch = 'E';
                        }
                    }
                }
                _ => {}
            }
            self.inter.clear();
            return;
        }
        match b {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => self.linefeed(),
            b'E' => {
                self.col = 0;
                self.linefeed();
            }
            b'M' => self.reverse_index(),
            b'H' => {
                if self.col < self.tabs.len() {
                    self.tabs[self.col] = true
                }
            }
            b'c' => self.full_reset(),
            b'=' => self.app_keypad = true,
            b'>' => self.app_keypad = false,
            b'\\' => {}
            _ => {}
        }
    }

    fn csi(&mut self, b: u8) {
        match b {
            b'0'..=b'9' => {
                self.cur = self.cur.saturating_mul(10).saturating_add((b - b'0') as u32).min(65535);
                self.has_cur = true;
            }
            b';' | b':' => {
                self.params.push(self.cur);
                self.sub.push(self.pending_sub());
                self.cur = 0;
                self.has_cur = false;
                // the next one is joined to this by a colon
                self.colon_next = b == b':';
            }
            b'<' | b'=' | b'>' | b'?' => {
                if self.params.is_empty() && !self.has_cur {
                    self.private = b;
                } else {
                    self.st = St::CsiIgnore;
                }
            }
            0x20..=0x2f => self.inter.push(b),
            0x40..=0x7e => {
                if self.has_cur || !self.params.is_empty() {
                    self.params.push(self.cur);
                    self.sub.push(self.pending_sub());
                }
                self.colon_next = false;
                self.st = St::Ground;
                self.csi_dispatch(b);
            }
            _ => self.st = St::CsiIgnore,
        }
    }

    fn pending_sub(&self) -> bool {
        self.colon_next
    }

    fn csi_dispatch(&mut self, b: u8) {
        let inter = self.inter.first().copied().unwrap_or(0);
        let p0 = self.param(0, 1) as usize;
        match (self.private, inter, b) {
            (0, 0, b'@') => self.insert_chars(p0),
            (0, 0, b'A') => {
                let lo = if self.row >= self.top { self.top } else { 0 };
                self.row = self.row.saturating_sub(p0).max(lo);
                self.pending = false;
            }
            (0, 0, b'B') | (0, 0, b'e') => {
                let hi = if self.row <= self.bottom { self.bottom } else { self.rows - 1 };
                self.row = (self.row + p0).min(hi);
                self.pending = false;
            }
            (0, 0, b'C') | (0, 0, b'a') => {
                self.col = (self.col + p0).min(self.cols - 1);
                self.pending = false;
            }
            (0, 0, b'D') => {
                self.col = self.col.saturating_sub(p0);
                self.pending = false;
            }
            (0, 0, b'E') => {
                let hi = if self.row <= self.bottom { self.bottom } else { self.rows - 1 };
                self.row = (self.row + p0).min(hi);
                self.col = 0;
                self.pending = false;
            }
            (0, 0, b'F') => {
                let lo = if self.row >= self.top { self.top } else { 0 };
                self.row = self.row.saturating_sub(p0).max(lo);
                self.col = 0;
                self.pending = false;
            }
            (0, 0, b'G') | (0, 0, b'`') => {
                self.col = (p0 - 1).min(self.cols - 1);
                self.pending = false;
            }
            (0, 0, b'H') | (0, 0, b'f') => {
                let r = self.param(0, 1) as i64 - 1;
                let c = self.param(1, 1) as i64 - 1;
                self.goto(r, c);
            }
            (0, 0, b'I') => {
                for _ in 0..p0 {
                    self.control(0x09);
                }
            }
            (0, 0, b'J') | (b'?', 0, b'J') => {
                let how = self.params.first().copied().unwrap_or(0);
                self.erase_display(how);
            }
            (0, 0, b'K') | (b'?', 0, b'K') => {
                let how = self.params.first().copied().unwrap_or(0);
                self.erase_line(how);
            }
            (0, 0, b'L') => self.insert_lines(p0),
            (0, 0, b'M') => self.delete_lines(p0),
            (0, 0, b'P') => self.delete_chars(p0),
            (0, 0, b'S') => self.scroll_up(p0),
            (0, 0, b'T') => self.scroll_down(p0),
            (0, 0, b'X') => {
                let (r, c) = (self.row, self.col);
                self.erase_cells(r, c, c + p0);
            }
            (0, 0, b'Z') => {
                for _ in 0..p0 {
                    let mut c = self.col;
                    while c > 0 {
                        c -= 1;
                        if self.tabs[c] {
                            break;
                        }
                    }
                    self.col = c;
                }
                self.pending = false;
            }
            (0, 0, b'b') => {
                let c = self.last;
                for _ in 0..p0.min(65535) {
                    self.put(c);
                }
            }
            (0, 0, b'c') => {
                if self.params.first().copied().unwrap_or(0) == 0 {
                    self.reply.push_str("\x1b[?62;22c");
                }
            }
            (b'>', 0, b'c') => self.reply.push_str("\x1b[>0;276;0c"),
            (0, 0, b'd') => {
                let c = self.col as i64;
                self.goto(p0 as i64 - 1, c);
                if self.origin {
                    // `goto` already took the region's top
                }
            }
            (0, 0, b'g') => match self.params.first().copied().unwrap_or(0) {
                0 => {
                    if self.col < self.tabs.len() {
                        self.tabs[self.col] = false
                    }
                }
                3 => self.tabs.iter_mut().for_each(|t| *t = false),
                _ => {}
            },
            (0, 0, b'h') | (b'?', 0, b'h') => self.set_mode(true),
            (0, 0, b'l') | (b'?', 0, b'l') => self.set_mode(false),
            (0, 0, b'm') => self.sgr(),
            (0, 0, b'n') => match self.params.first().copied().unwrap_or(0) {
                5 => self.reply.push_str("\x1b[0n"),
                6 => {
                    let r = if self.origin { self.row - self.top } else { self.row };
                    self.reply.push_str(&format!("\x1b[{};{}R", r + 1, self.col + 1));
                }
                _ => {}
            },
            (b'?', 0, b'n') => {
                if self.params.first().copied() == Some(6) {
                    self.reply.push_str(&format!("\x1b[?{};{}R", self.row + 1, self.col + 1));
                }
            }
            (0, b' ', b'q') => self.cursor_style = self.params.first().copied().unwrap_or(0).min(6) as u8,
            (0, 0, b'r') => {
                let t = self.param(0, 1) as usize;
                let bt = self.param(1, self.rows as u32) as usize;
                if t < bt && bt <= self.rows {
                    self.top = t - 1;
                    self.bottom = bt - 1;
                    self.goto(0, 0);
                }
            }
            (0, 0, b's') => self.save_cursor(),
            (0, 0, b'u') => self.restore_cursor(),
            (0, 0, b't') => {
                if self.params.first().copied() == Some(18) {
                    self.reply.push_str(&format!("\x1b[8;{};{}t", self.rows, self.cols));
                }
            }
            (0, b'!', b'p') => self.soft_reset(),
            (pr, b'$', b'p') => {
                let p = self.params.first().copied().unwrap_or(0);
                let v = self.mode_state(p);
                let q = if pr == b'?' { "?" } else { "" };
                self.reply.push_str(&format!("\x1b[{q}{p};{v}$y"));
            }
            _ => {}
        }
        self.params.clear();
        self.sub.clear();
        self.inter.clear();
        self.private = 0;
    }

    fn osc_end(&mut self) {
        self.st = St::Ground;
        let raw = std::mem::take(&mut self.osc);
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (code, rest) = match text.find(';') {
            Some(i) => (&text[..i], &text[i + 1..]),
            None => (text.as_str(), ""),
        };
        match code {
            "0" | "2" => self.title = rest.chars().take(256).collect(),
            "7" => {
                // file://host/path, percent-encoded
                let path = rest.strip_prefix("file://").map(|r| match r.find('/') {
                    Some(i) => &r[i..],
                    None => "",
                });
                if let Some(p) = path {
                    self.cwd = percent_decode(p);
                }
            }
            "8" => {
                // 8;params;uri — an empty uri ends the link
                let uri = match rest.find(';') {
                    Some(i) => &rest[i + 1..],
                    None => "",
                };
                if uri.is_empty() {
                    self.link = 0;
                } else if let Some(i) = self.links.iter().rposition(|l| l == uri) {
                    self.link = (i + 1) as u16;
                } else {
                    if self.links.len() >= 4096 {
                        self.links.clear();
                    }
                    self.links.push(uri.to_string());
                    self.link = self.links.len() as u16;
                }
            }
            "10" | "11" if rest == "?" => {
                let (r, g, b) = if code == "10" { self.fg_rgb } else { self.bg_rgb };
                self.reply.push_str(&format!("\x1b]{code};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x1b\\"));
            }
            _ => {}
        }
    }

    // ── resize ──

    fn resize(&mut self, rows: usize, cols: usize) {
        let rows = rows.clamp(1, 1000);
        let cols = cols.clamp(1, 2000);
        if rows == self.rows && cols == self.cols {
            return;
        }
        // the alternate screen: cut or padded, never rewrapped
        if let Some(alt) = self.alt.as_mut() {
            for l in alt.iter_mut() {
                l.cells.resize(cols, BLANK);
                if l.cells[cols - 1].w == 2 {
                    l.cells[cols - 1] = BLANK;
                }
            }
            while alt.len() > rows {
                alt.remove(0);
            }
            while alt.len() < rows {
                alt.push(Line::new(cols, Attr::default()));
            }
            self.saved_main.row = self.saved_main.row.min(rows - 1);
            self.saved_main.col = self.saved_main.col.min(cols - 1);
            self.reflow_main(rows, cols, None);
            let shift = self.rows.saturating_sub(rows);
            self.row = self.row.saturating_sub(shift).min(rows - 1);
            self.col = self.col.min(cols - 1);
        } else {
            let cur = (self.row, self.col);
            let (r, c) = self.reflow_main(rows, cols, Some(cur));
            self.row = r;
            self.col = c;
        }
        self.rows = rows;
        self.cols = cols;
        self.top = 0;
        self.bottom = rows - 1;
        self.pending = false;
        let mut tabs = Term::default_tabs(cols);
        for (i, t) in self.tabs.iter().enumerate() {
            if i < cols {
                tabs[i] = *t;
            }
        }
        self.tabs = tabs;
    }

    /// The main screen and its scrollback rewrapped at `cols`, the screen
    /// `rows` high; the cursor `cur` (row, col) where its text went.
    fn reflow_main(&mut self, rows: usize, cols: usize, cur: Option<(usize, usize)>) -> (usize, usize) {
        let old_rows = self.rows;
        let base = self.lines.len() - old_rows;
        let cur_abs = cur.map(|(r, c)| (base + r, c));
        // the content: through the cursor's line, or the last line not blank
        let mut end = self.lines.len();
        let floor = cur_abs.map(|(i, _)| i + 1).unwrap_or(0);
        while end > floor && self.lines[end - 1].is_blank() {
            end -= 1;
        }
        if cur.is_none() {
            // the main screen behind the alternate one: its cursor saved
            let r = base + self.saved_main.row;
            end = end.max((r + 1).min(self.lines.len()));
        }
        let same_cols = cols == self.cols;
        let mut out: Vec<Line> = Vec::with_capacity(end + rows);
        let mut new_cur: Option<(usize, usize)> = None;
        let mut i = 0;
        while i < end {
            // one logical line: physical lines joined while wrapped
            let mut cells: Vec<Cell> = Vec::new();
            let mut extra: Vec<(usize, String)> = Vec::new();
            let mut cur_off: Option<usize> = None;
            loop {
                let l = &self.lines[i];
                let take = if l.wrapped && i + 1 < end { l.cells.len() } else { l.trimmed_len() };
                if let Some((ci, cc)) = cur_abs
                    && ci == i
                {
                    cur_off = Some(cells.len() + cc);
                }
                if let Some(e) = &l.extra {
                    for (c, t) in e.iter() {
                        if (*c as usize) < take {
                            extra.push((cells.len() + *c as usize, t.clone()));
                        }
                    }
                }
                cells.extend_from_slice(&l.cells[..take.min(l.cells.len())]);
                let wrapped = l.wrapped && i + 1 < end;
                i += 1;
                if !wrapped {
                    break;
                }
            }
            if same_cols && cells.len() <= cols {
                // most lines: as they were
                let start = out.len();
                let mut l = Line {
                    cells,
                    wrapped: false,
                    extra: None,
                };
                if !extra.is_empty() {
                    l.extra = Some(Box::new(extra.into_iter().map(|(c, t)| (c as u16, t)).collect()));
                }
                l.cells.resize(cols, BLANK);
                out.push(l);
                if let Some(off) = cur_off {
                    new_cur = Some((start, off.min(cols - 1)));
                }
                continue;
            }
            // rewrapped at `cols`
            let start_row = out.len();
            let mut p = 0;
            let mut chunks: Vec<(usize, usize)> = Vec::new();
            if cells.is_empty() {
                chunks.push((0, 0));
            }
            while p < cells.len() {
                let mut q = (p + cols).min(cells.len());
                if q < cells.len() && q > p + 1 && cells[q - 1].w == 2 {
                    q -= 1;
                }
                chunks.push((p, q));
                p = q;
            }
            let n = chunks.len();
            for (k, (a, b)) in chunks.iter().enumerate() {
                let mut l = Line {
                    cells: cells[*a..*b].to_vec(),
                    wrapped: k + 1 < n,
                    extra: None,
                };
                let ex: Vec<(u16, String)> = extra.iter().filter(|(c, _)| c >= a && c < b).map(|(c, t)| ((c - a) as u16, t.clone())).collect();
                if !ex.is_empty() {
                    l.extra = Some(Box::new(ex));
                }
                if l.cells.first().map(|c| c.w == 0).unwrap_or(false) {
                    l.cells[0] = BLANK;
                }
                l.cells.resize(cols, BLANK);
                out.push(l);
            }
            if let Some(off) = cur_off {
                let (k, a) = chunks
                    .iter()
                    .enumerate()
                    .find(|(_, (a, b))| off >= *a && off < *b)
                    .map(|(k, (a, _))| (k, *a))
                    .unwrap_or((n - 1, chunks[n - 1].0));
                let col = off - a;
                if col >= cols {
                    // past the text (spaces typed, then the line shortened)
                    let extra_rows = col / cols;
                    for _ in 0..extra_rows {
                        out.push(Line::new(cols, Attr::default()));
                    }
                    new_cur = Some((start_row + k + extra_rows, col % cols));
                } else {
                    new_cur = Some((start_row + k, col));
                }
            }
        }
        // the cursor on a line past the content (blank lines below text)
        let (cur_line, cur_col) = match (new_cur, cur_abs) {
            (Some(x), _) => x,
            (None, Some((ci, cc))) => {
                while out.len() <= ci.min(end) {
                    out.push(Line::new(cols, Attr::default()));
                }
                (out.len() - 1, cc.min(cols - 1))
            }
            (None, None) => (out.len().saturating_sub(1), 0),
        };
        while out.len() < rows {
            out.push(Line::new(cols, Attr::default()));
        }
        // the screen ends `rows` below its start, the cursor on it
        while out.len() > rows && out.len() - rows > cur_line && out.last().map(|l| l.is_blank()).unwrap_or(false) {
            out.pop();
        }
        while out.len() - rows > cur_line {
            out.pop();
        }
        self.lines = out.into();
        let mut cur_line = cur_line;
        while self.lines.len() - rows > self.max_scrollback {
            self.lines.pop_front();
            self.evicted += 1;
            cur_line = cur_line.saturating_sub(1);
        }
        let r = cur_line.saturating_sub(self.lines.len() - rows).min(rows - 1);
        (r, cur_col.min(cols - 1))
    }
}

fn percent_decode(t: &str) -> String {
    let b = t.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&t[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── the registry ─────────────────────────────────────────────────────

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn screens() -> &'static Mutex<HashMap<i64, Arc<Mutex<Term>>>> {
    static SCREENS: OnceLock<Mutex<HashMap<i64, Arc<Mutex<Term>>>>> = OnceLock::new();
    SCREENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn handle(id: i64) -> Value {
    let mut fields = crate::ast::ValueMap::default();
    fields.insert("id".to_string(), Value::Integer(id));
    Value::Struct {
        type_name: "Screen".to_string(),
        fields: Arc::new(fields),
    }
}

fn screen_of(v: Option<&Value>, who: &str) -> Result<Arc<Mutex<Term>>, String> {
    let id = match v {
        Some(Value::Struct { type_name, fields }) if type_name == "Screen" => match fields.get("id") {
            Some(Value::Integer(i)) => *i,
            _ => return Err(format!("{who}: a malformed Screen handle")),
        },
        Some(other) => return Err(format!("{who}: expected a Screen, got {}", other.type_name())),
        None => return Err(format!("{who}: expected a Screen")),
    };
    screens().lock().unwrap().get(&id).cloned().ok_or_else(|| format!("{who}: this Screen was freed"))
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Integer(i)) => Some(*i as f64),
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    }
}

fn fields(v: Option<&Value>) -> Option<&crate::ast::ValueMap> {
    match v {
        Some(Value::Map(m)) => Some(m),
        Some(Value::Struct { fields, .. }) => Some(fields),
        _ => None,
    }
}

fn opt_num(o: Option<&crate::ast::ValueMap>, k: &str) -> Option<f64> {
    num(o.and_then(|m| m.get(k)))
}

fn rgb_of(v: Option<&Value>) -> Option<(u8, u8, u8)> {
    match v? {
        Value::String(t) => {
            let h = t.strip_prefix('#')?;
            if h.len() < 6 {
                return None;
            }
            let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
            Some((p(0)?, p(2)?, p(4)?))
        }
        Value::Tuple(xs) | Value::List(xs) if xs.len() >= 3 => {
            let n = |v: &Value| num(Some(v)).map(|f| f.clamp(0.0, 255.0) as u8);
            Some((n(&xs[0])?, n(&xs[1])?, n(&xs[2])?))
        }
        _ => None,
    }
}

fn line_col(v: Option<&Value>) -> i64 {
    num(v).unwrap_or(0.0) as i64
}

pub fn call_vt_function(name: &str, args: Vec<Value>) -> Res {
    let who = format!("vt.{name}");
    if name == "new" {
        let rows = num(args.first()).unwrap_or(24.0) as usize;
        let cols = num(args.get(1)).unwrap_or(80.0) as usize;
        let o = fields(args.get(2));
        let sb = opt_num(o, "scrollback").unwrap_or(10_000.0).clamp(0.0, 1_000_000.0) as usize;
        let mut t = Term::new(rows, cols, sb);
        if let Some(c) = rgb_of(o.and_then(|m| m.get("fg"))) {
            t.fg_rgb = c;
        }
        if let Some(c) = rgb_of(o.and_then(|m| m.get("bg"))) {
            t.bg_rgb = c;
        }
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        screens().lock().unwrap().insert(id, Arc::new(Mutex::new(t)));
        return Ok(handle(id));
    }
    if name == "free" {
        if let Some(Value::Struct { fields, .. }) = args.first()
            && let Some(Value::Integer(i)) = fields.get("id")
        {
            screens().lock().unwrap().remove(i);
        }
        return Ok(Value::Unit);
    }
    // a reader task may feed a screen a moment after the program let it
    // go (its terminal closed): nothing to do, not an error
    let sc = match screen_of(args.first(), &who) {
        Ok(sc) => sc,
        Err(e) => {
            return match name {
                "feed" => Ok(s("")),
                "claim" => Ok(Value::Boolean(false)),
                _ => Err(e.into()),
            };
        }
    };
    let mut t = sc.lock().unwrap_or_else(|e| e.into_inner());
    match name {
        "feed" => {
            match args.get(1) {
                Some(Value::String(x)) => t.feed(x.as_bytes()),
                Some(other) => match crate::stdlib::bytes::bytes_of(other) {
                    Ok(b) => t.feed(b),
                    Err(_) => return Err(format!("{who}: data is a String or Bytes, got {}", other.type_name()).into()),
                },
                None => return Err(crate::stdlib::misuse::arity("vt.feed", "2", args.len())),
            }
            let r = std::mem::take(&mut t.reply);
            Ok(s(&r))
        }
        "resize" => {
            let rows = num(args.get(1)).unwrap_or(t.rows as f64) as usize;
            let cols = num(args.get(2)).unwrap_or(t.cols as f64) as usize;
            t.resize(rows, cols);
            t.rev += 1;
            Ok(Value::Unit)
        }
        "claim" => {
            let was = t.notified;
            t.notified = true;
            Ok(Value::Boolean(!was))
        }
        "info" => {
            t.notified = false;
            let cur_id = t.screen_id() + t.row as i64;
            let mouse = match t.mouse {
                9 | 1000 => "click",
                1002 => "drag",
                1003 => "motion",
                _ => "",
            };
            let style = match t.cursor_style {
                3 | 4 => "underline",
                5 | 6 => "bar",
                _ => "block",
            };
            Ok(obj("TermInfo", vec![
                ("rows", int(t.rows as i64)),
                ("cols", int(t.cols as i64)),
                ("title", s(&t.title)),
                ("cwd", s(&t.cwd)),
                ("alt", Value::Boolean(t.alt.is_some())),
                ("cursor", tuple(vec![int(cur_id), int(t.col as i64)])),
                ("cursor_row", int(t.row as i64)),
                ("cursor_visible", Value::Boolean(t.cursor_visible)),
                ("cursor_style", s(style)),
                ("cursor_blink", Value::Boolean(t.cursor_style == 0 || t.cursor_style % 2 == 1)),
                ("app_cursor", Value::Boolean(t.app_cursor)),
                ("app_keypad", Value::Boolean(t.app_keypad)),
                ("bracketed_paste", Value::Boolean(t.bracketed)),
                ("mouse", s(mouse)),
                ("mouse_sgr", Value::Boolean(t.mouse_sgr)),
                ("focus_events", Value::Boolean(t.focus_events)),
                ("reverse", Value::Boolean(t.reverse)),
                ("bell", int(t.bell)),
                ("rev", int(t.rev)),
                ("first", int(t.first_id())),
                ("screen", int(t.screen_id())),
                ("last", int(t.last_id())),
            ]))
        }
        "render" => {
            let o = fields(args.get(1));
            Ok(Value::List(Arc::new(render(&t, o))))
        }
        "text" => {
            let (l0, c0, l1, c1) = (line_col(args.get(1)), line_col(args.get(2)), line_col(args.get(3)), line_col(args.get(4)));
            Ok(s(&text_of(&t, (l0, c0), (l1, c1))))
        }
        "line" => {
            let id = line_col(args.get(1));
            match t.line_by_id(id) {
                None => Ok(Value::Unit),
                Some(l) => {
                    // the text, and the column each character starts at
                    let mut text = String::new();
                    let mut starts = Vec::new();
                    let n = l.trimmed_len();
                    for c in 0..n {
                        let cell = &l.cells[c];
                        if cell.w == 0 {
                            continue;
                        }
                        text.push(cell.ch);
                        starts.push(int(c as i64));
                        if let Some(x) = l.extra_at(c) {
                            for ch in x.chars() {
                                text.push(ch);
                                starts.push(int(c as i64));
                            }
                        }
                    }
                    Ok(obj("TermLine", vec![
                        ("text", s(&text)),
                        ("cols", Value::List(Arc::new(starts))),
                        ("wrapped", Value::Boolean(l.wrapped)),
                    ]))
                }
            }
        }
        "word_at" => {
            let id = line_col(args.get(1));
            let col = line_col(args.get(2)).max(0) as usize;
            match t.line_by_id(id) {
                None => Ok(Value::Unit),
                Some(l) => {
                    let word = |c: &Cell| c.w == 0 || c.ch.is_alphanumeric() || "-_./~:@%+#?=&".contains(c.ch);
                    if col >= l.cells.len() || !word(&l.cells[col]) || l.cells[col].ch == ' ' {
                        return Ok(tuple(vec![int(col as i64), int(col as i64 + 1)]));
                    }
                    let mut a = col;
                    while a > 0 && word(&l.cells[a - 1]) && l.cells[a - 1].ch != ' ' {
                        a -= 1;
                    }
                    let mut b = col + 1;
                    while b < l.cells.len() && word(&l.cells[b]) && !(l.cells[b].ch == ' ' && l.cells[b].w != 0) {
                        b += 1;
                    }
                    Ok(tuple(vec![int(a as i64), int(b as i64)]))
                }
            }
        }
        "link_at" => {
            let id = line_col(args.get(1));
            let col = line_col(args.get(2)).max(0) as usize;
            let l = match t.line_by_id(id) {
                Some(l) => l,
                None => return Ok(Value::Unit),
            };
            let k = l.cells.get(col).map(|c| c.a.link).unwrap_or(0);
            if k == 0 {
                return Ok(Value::Unit);
            }
            let mut a = col;
            while a > 0 && l.cells[a - 1].a.link == k {
                a -= 1;
            }
            let mut b = col + 1;
            while b < l.cells.len() && l.cells[b].a.link == k {
                b += 1;
            }
            Ok(obj("TermLink", vec![
                ("uri", s(t.links.get(k as usize - 1).map(|x| x.as_str()).unwrap_or(""))),
                ("from", int(a as i64)),
                ("to", int(b as i64)),
            ]))
        }
        "find" => {
            let q = match args.get(1) {
                Some(Value::String(q)) => q.as_ref().clone(),
                _ => String::new(),
            };
            let o = fields(args.get(2));
            let case = matches!(o.and_then(|m| m.get("case")), Some(Value::Boolean(true)));
            Ok(Value::List(Arc::new(find(&t, &q, case))))
        }
        "clear" => {
            // ⌘K: the scrollback gone, the cursor's line the screen's first
            if t.alt.is_none() {
                let row = t.row;
                let base = t.screen_base();
                let keep = t.lines[base + row].clone();
                let n = t.lines.len();
                t.evicted += n as i64;
                t.lines.clear();
                t.lines.push_back(keep);
                let (rows, cols) = (t.rows, t.cols);
                for _ in 1..rows {
                    t.lines.push_back(Line::new(cols, Attr::default()));
                }
                t.row = 0;
            }
            t.rev += 1;
            Ok(Value::Unit)
        }
        "reset" => {
            t.full_reset();
            t.rev += 1;
            Ok(Value::Unit)
        }
        "set_colors" => {
            let o = fields(args.get(1));
            if let Some(c) = rgb_of(o.and_then(|m| m.get("fg"))) {
                t.fg_rgb = c;
            }
            if let Some(c) = rgb_of(o.and_then(|m| m.get("bg"))) {
                t.bg_rgb = c;
            }
            Ok(Value::Unit)
        }
        _ => Err(format!("Unknown vt function: {name}").into()),
    }
}

/// The text from `(l0, c0)` to `(l1, c1)` (inclusive of the first cell,
/// exclusive of the last): lines that wrapped joined, the others ended
/// with a newline, trailing blanks dropped.
fn text_of(t: &Term, a: (i64, i64), b: (i64, i64)) -> String {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let mut out = String::new();
    let mut id = a.0;
    while id <= b.0 {
        if let Some(l) = t.line_by_id(id) {
            let from = if id == a.0 { a.1.max(0) as usize } else { 0 };
            let to = if id == b.0 { (b.1.max(0) as usize).min(l.cells.len()) } else { l.cells.len() };
            let to = if id == b.0 || !l.wrapped { to.min(l.trimmed_len().max(from)) } else { to };
            if from < to {
                out.push_str(&l.text(from, to));
            }
            if id < b.0 && !l.wrapped {
                out.push('\n');
            }
        }
        id += 1;
    }
    out
}

fn find(t: &Term, q: &str, case: bool) -> Vec<Value> {
    if q.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = if case { q.chars().collect() } else { q.to_lowercase().chars().collect() };
    let low = |c: char| if case { c } else { c.to_lowercase().next().unwrap_or(c) };
    let first = if t.alt.is_some() { t.screen_id() } else { t.first_id() };
    // Logical lines (a line that wrapped joined to the next), from the
    // end back: a match may cross a wrap, and the newest 2000 are kept.
    let mut found: Vec<(i64, i64, i64)> = Vec::new();
    let mut end = t.last_id();
    while end >= first && found.len() < 2000 {
        let mut start = end;
        while start > first && t.line_by_id(start - 1).map(|l| l.wrapped).unwrap_or(false) {
            start -= 1;
        }
        // its characters with their line and column
        let mut chars: Vec<(char, i64, usize)> = Vec::new();
        for id in start..=end {
            if let Some(l) = t.line_by_id(id) {
                for (c, cell) in l.cells.iter().enumerate() {
                    if cell.w != 0 {
                        chars.push((low(cell.ch), id, c));
                    }
                }
            }
        }
        let mut hits: Vec<(i64, i64, i64)> = Vec::new();
        let mut i = 0;
        while i + needle.len() <= chars.len() {
            if chars[i..i + needle.len()].iter().zip(needle.iter()).all(|((c, _, _), n)| c == n) {
                let (_, l0, c0) = chars[i];
                let (_, l1, c1) = chars[i + needle.len() - 1];
                let w1 = t.line_by_id(l1).and_then(|l| l.cells.get(c1)).map(|c| c.w.max(1) as i64).unwrap_or(1);
                // cells from the first to past the last, across the wraps
                let cells = (l1 - l0) * t.cols as i64 + c1 as i64 + w1 - c0 as i64;
                hits.push((l0, c0 as i64, cells));
                i += needle.len();
            } else {
                i += 1;
            }
        }
        found.extend(hits.into_iter().rev());
        end = start - 1;
    }
    found.truncate(2000);
    found.into_iter().rev().map(|(l, c, w)| tuple(vec![int(l), int(c), int(w)])).collect()
}

// ── drawing ──────────────────────────────────────────────────────────

/// xterm's 256 colours past the 16 the palette gives.
fn color256(n: u32) -> (u8, u8, u8) {
    if n >= 232 {
        let v = (8 + (n - 232) * 10) as u8;
        return (v, v, v);
    }
    let n = n - 16;
    let step = |x: u32| if x == 0 { 0u8 } else { (55 + x * 40) as u8 };
    (step(n / 36), step((n / 6) % 6), step(n % 6))
}

struct Look {
    pal: [(u8, u8, u8); 16],
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    cursor: (u8, u8, u8),
    cursor_text: (u8, u8, u8),
    sel: (u8, u8, u8),
    find: (u8, u8, u8),
    find_cur: (u8, u8, u8),
    link: (u8, u8, u8),
}

const XTERM16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn look_of(o: Option<&crate::ast::ValueMap>) -> Look {
    let g = |k: &str, d: (u8, u8, u8)| rgb_of(o.and_then(|m| m.get(k))).unwrap_or(d);
    let mut pal = XTERM16;
    if let Some(Value::List(xs)) | Some(Value::Tuple(xs)) = o.and_then(|m| m.get("palette")) {
        for (i, v) in xs.iter().take(16).enumerate() {
            if let Some(c) = rgb_of(Some(v)) {
                pal[i] = c;
            }
        }
    }
    let fg = g("fg", (0xd4, 0xd4, 0xd8));
    let bg = g("bg", (0x18, 0x18, 0x1b));
    Look {
        pal,
        fg,
        bg,
        cursor: g("cursor_color", fg),
        cursor_text: g("cursor_text", bg),
        sel: g("selection_color", (0x3a, 0x5a, 0x8a)),
        find: g("find_color", (0x6a, 0x5a, 0x20)),
        find_cur: g("find_current_color", (0xb0, 0x80, 0x10)),
        link: g("link_color", fg),
    }
}

fn rgb_val(c: (u8, u8, u8)) -> Value {
    tuple(vec![int(c.0 as i64), int(c.1 as i64), int(c.2 as i64)])
}

fn resolve(look: &Look, c: u32, fg: bool) -> (u8, u8, u8) {
    if c == 0 {
        return if fg { look.fg } else { look.bg };
    }
    if c & RGB != 0 {
        return (((c >> 16) & 0xff) as u8, ((c >> 8) & 0xff) as u8, (c & 0xff) as u8);
    }
    let n = c & 0xff;
    if n < 16 { look.pal[n as usize] } else { color256(n) }
}

fn lum(c: (u8, u8, u8)) -> f32 {
    let f = |x: u8| {
        let v = x as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
}

fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `fg` moved toward black or white (whichever the background leaves more
/// room for) until it holds `want` against `bg`; itself when it does.
fn legible_on(fg: (u8, u8, u8), bg: (u8, u8, u8), want: f32) -> (u8, u8, u8) {
    if contrast(fg, bg) >= want {
        return fg;
    }
    let to = if lum(bg) > 0.18 { (0, 0, 0) } else { (255, 255, 255) };
    let mut k = 0.1;
    while k < 1.0 {
        let c = mix(fg, to, k);
        if contrast(c, bg) >= want {
            return c;
        }
        k += 0.1;
    }
    to
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), k: f32) -> (u8, u8, u8) {
    let m = |x: u8, y: u8| (x as f32 * (1.0 - k) + y as f32 * k).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Characters drawn together in one text: those the code font has at
/// one cell each (ASCII, Latin, box drawing and blocks).
fn groups(c: char) -> bool {
    let u = c as u32;
    (0x20..0x7f).contains(&u) || (0xa0..0x250).contains(&u) || (0x2500..0x25a0).contains(&u)
}

fn render(t: &Term, o: Option<&crate::ast::ValueMap>) -> Vec<Value> {
    let look = look_of(o);
    let cw = opt_num(o, "cell_w").unwrap_or(8.0) as f32;
    let ch = opt_num(o, "cell_h").unwrap_or(17.0) as f32;
    let size = opt_num(o, "size").unwrap_or(13.0) as f32;
    let px = opt_num(o, "pad_x").unwrap_or(0.0) as f32;
    let py = opt_num(o, "pad_y").unwrap_or(0.0) as f32;
    let text_dy = opt_num(o, "text_dy").unwrap_or(((ch - size * 1.3) / 2.0) as f64) as f32;
    let show_rows = opt_num(o, "rows").map(|r| r as usize).unwrap_or(t.rows).max(1);
    let bold_w = opt_num(o, "bold").unwrap_or(700.0);
    let min_contrast = opt_num(o, "min_contrast").unwrap_or(0.0) as f32;
    let mut legible: HashMap<((u8, u8, u8), (u8, u8, u8), bool), (u8, u8, u8)> = HashMap::new();
    let font = match o.and_then(|m| m.get("font")) {
        Some(Value::String(f)) => f.as_ref().clone(),
        _ => "mono".to_string(),
    };
    // the first line drawn: `top` (a line's number), else the screen's
    let bottom_top = t.screen_id() + t.rows as i64 - show_rows as i64;
    let top = match opt_num(o, "top") {
        Some(v) if t.alt.is_none() => (v as i64).clamp(t.first_id(), bottom_top.max(t.first_id())),
        _ => bottom_top,
    };
    let sel = match o.and_then(|m| m.get("selection")) {
        Some(Value::Tuple(xs)) | Some(Value::List(xs)) if xs.len() >= 4 => {
            let v: Vec<i64> = xs.iter().map(|x| num(Some(x)).unwrap_or(0.0) as i64).collect();
            let (a, b) = ((v[0], v[1]), (v[2], v[3]));
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            Some((a, b))
        }
        _ => None,
    };
    // find's matches: (line, col, cells); the current one apart
    let mut marks: Vec<(i64, i64, i64, bool)> = Vec::new();
    if let Some(Value::List(xs)) = o.and_then(|m| m.get("matches")) {
        let cur = opt_num(o, "match").map(|c| c as i64).unwrap_or(-1);
        for (i, m) in xs.iter().enumerate() {
            if let Value::Tuple(v) | Value::List(v) = m
                && v.len() >= 3
            {
                // a match that crosses a wrap goes on at the next line's start
                let mut l = num(v.first()).unwrap_or(0.0) as i64;
                let mut c0 = num(v.get(1)).unwrap_or(0.0) as i64;
                let mut w = num(v.get(2)).unwrap_or(0.0) as i64;
                let cols = t.cols as i64;
                while w > 0 && l < top + show_rows as i64 {
                    let seg = w.min(cols - c0).max(0);
                    if l >= top && seg > 0 {
                        marks.push((l, c0, seg, i as i64 == cur));
                    }
                    w -= seg;
                    l += 1;
                    c0 = 0;
                }
            }
        }
    }
    let hover = match o.and_then(|m| m.get("hover")) {
        Some(Value::Tuple(xs)) | Some(Value::List(xs)) if xs.len() >= 3 => {
            Some((num(xs.first()).unwrap_or(0.0) as i64, num(xs.get(1)).unwrap_or(0.0) as i64, num(xs.get(2)).unwrap_or(0.0) as i64))
        }
        _ => None,
    };
    let cursor_on = !matches!(o.and_then(|m| m.get("cursor")), Some(Value::Boolean(false)));
    let hollow = matches!(o.and_then(|m| m.get("hollow")), Some(Value::Boolean(true)));
    let style_over = match o.and_then(|m| m.get("cursor_style")) {
        Some(Value::String(x)) => Some(x.as_ref().clone()),
        _ => None,
    };
    let preedit = match o.and_then(|m| m.get("preedit")) {
        Some(Value::String(x)) if !x.is_empty() => Some(x.as_ref().clone()),
        _ => None,
    };

    let mut ops: Vec<Value> = Vec::with_capacity(show_rows * 6);
    let rect = |x: f32, y: f32, w: f32, h: f32, c: (u8, u8, u8)| {
        vmap(vec![("op", s("rect")), ("x", fl(x)), ("y", fl(y)), ("w", fl(w)), ("h", fl(h)), ("fill", rgb_val(c))])
    };
    let in_sel = |id: i64, c: i64| -> bool {
        match sel {
            None => false,
            Some((a, b)) => (id, c) >= a && (id, c) < b,
        }
    };
    let cur_id = t.screen_id() + t.row as i64;
    let rev = t.reverse;

    for r in 0..show_rows {
        let id = top + r as i64;
        let Some(line) = t.line_by_id(id) else { continue };
        let y = py + r as f32 * ch;
        let n = line.cells.len().min(t.cols);
        // each cell's colours, as drawn
        let mut fgs: Vec<(u8, u8, u8)> = Vec::with_capacity(n);
        let mut bgs: Vec<Option<(u8, u8, u8)>> = Vec::with_capacity(n);
        for c in 0..n {
            let cell = &line.cells[c];
            let a = if cell.w == 0 && c > 0 { line.cells[c - 1].a } else { cell.a };
            let mut f = resolve(&look, a.fg, true);
            let mut b = if a.bg == 0 { None } else { Some(resolve(&look, a.bg, false)) };
            if a.fl & BOLD != 0 && a.fg & IDX != 0 && (a.fg & 0xff) < 8 && a.fg & RGB == 0 {
                // bold keeps its colour (no brighter one): the weight says it
            }
            if (a.fl & INVERSE != 0) != rev {
                let bb = b.unwrap_or(look.bg);
                b = Some(f);
                f = bb;
            }
            if a.fl & DIM != 0 {
                f = mix(f, b.unwrap_or(look.bg), 0.4);
            }
            if in_sel(id, c as i64) {
                b = Some(look.sel);
            }
            fgs.push(f);
            bgs.push(b);
        }
        for (l, c0, w, cur) in &marks {
            if *l == id {
                for c in (*c0).max(0)..(*c0 + *w).min(n as i64) {
                    bgs[c as usize] = Some(if *cur { look.find_cur } else { look.find });
                }
            }
        }
        // each character legible on what is under it (a program's white on
        // a light background, a selection, a match): moved toward black or
        // white until it holds `min_contrast` (dim text a little less)
        if min_contrast > 1.0 {
            for c in 0..n {
                let b = bgs[c].unwrap_or(look.bg);
                let a = line.cells[c].a;
                let want = if a.fl & DIM != 0 { (min_contrast * 0.66).max(3.0) } else { min_contrast };
                let key = (fgs[c], b, a.fl & DIM != 0);
                let f = *legible.entry(key).or_insert_with(|| legible_on(fgs[c], b, want));
                fgs[c] = f;
            }
        }
        // backgrounds, a rectangle a run
        let mut c = 0;
        while c < n {
            match bgs[c] {
                None => c += 1,
                Some(b) => {
                    let s0 = c;
                    while c < n && bgs[c] == Some(b) {
                        c += 1;
                    }
                    ops.push(rect(px + s0 as f32 * cw, y, (c - s0) as f32 * cw, ch, b));
                }
            }
        }
        // text, a text a run of one look
        let mut c = 0;
        while c < n {
            let cell = &line.cells[c];
            if cell.w == 0 || (cell.ch == ' ' && line.extra_at(c).is_none()) || cell.a.fl & HIDDEN != 0 {
                c += 1;
                continue;
            }
            let key = (fgs[c], cell.a.fl & (BOLD | ITALIC));
            let s0 = c;
            let mut txt = String::new();
            if !groups(cell.ch) || cell.w == 2 || line.extra_at(c).is_some() {
                txt.push(cell.ch);
                if let Some(x) = line.extra_at(c) {
                    txt.push_str(x);
                }
                c += cell.w.max(1) as usize;
            } else {
                while c < n {
                    let k = &line.cells[c];
                    if k.w != 1 || !groups(k.ch) || line.extra_at(c).is_some() || k.a.fl & HIDDEN != 0 || (fgs[c], k.a.fl & (BOLD | ITALIC)) != key {
                        break;
                    }
                    txt.push(k.ch);
                    c += 1;
                }
                let trimmed = txt.trim_end().len();
                txt.truncate(trimmed);
            }
            if txt.is_empty() {
                continue;
            }
            let mut fields = vec![
                ("op", s("text")),
                ("x", fl(px + s0 as f32 * cw)),
                ("y", fl(y + text_dy)),
                ("text", s(&txt)),
                ("size", fl(size)),
                ("font", s(&font)),
                ("color", rgb_val(key.0)),
            ];
            if key.1 & BOLD != 0 {
                fields.push(("weight", Value::Float(bold_w)));
            }
            if key.1 & ITALIC != 0 {
                fields.push(("italic", Value::Boolean(true)));
            }
            ops.push(vmap(fields));
        }
        // underlines and strikes (and a hyperlink under the pointer)
        let mut c = 0;
        while c < n {
            let a = line.cells[c].a;
            let hov = hover.map(|(l, a0, a1)| l == id && (c as i64) >= a0 && (c as i64) < a1).unwrap_or(false);
            let ul = a.fl & UNDERLINE != 0 || hov;
            let st = a.fl & STRIKE != 0;
            if !ul && !st {
                c += 1;
                continue;
            }
            let s0 = c;
            let f = fgs[c];
            while c < n {
                let a2 = line.cells[c].a;
                let hov2 = hover.map(|(l, a0, a1)| l == id && (c as i64) >= a0 && (c as i64) < a1).unwrap_or(false);
                if (a2.fl & UNDERLINE != 0 || hov2) != ul || (a2.fl & STRIKE != 0) != st || fgs[c] != f {
                    break;
                }
                c += 1;
            }
            let x = px + s0 as f32 * cw;
            let w = (c - s0) as f32 * cw;
            if ul {
                ops.push(rect(x, y + ch - 2.0, w, 1.0, if hov { look.link } else { f }));
            }
            if st {
                ops.push(rect(x, y + (ch / 2.0).round(), w, 1.0, f));
            }
        }
        // the cursor
        if id == cur_id && cursor_on && t.cursor_visible {
            let col = t.col.min(t.cols - 1);
            let x = px + col as f32 * cw;
            let wide = line.cells.get(col).map(|c| c.w == 2).unwrap_or(false);
            let w = if wide { cw * 2.0 } else { cw };
            let style = style_over.clone().unwrap_or_else(|| match t.cursor_style {
                3 | 4 => "underline".to_string(),
                5 | 6 => "bar".to_string(),
                _ => "block".to_string(),
            });
            if let Some(pe) = &preedit {
                // the input method's text at the cursor, underlined
                let pw = pe.chars().map(|c| c.width().unwrap_or(1).max(1)).sum::<usize>() as f32 * cw;
                ops.push(rect(x, y, pw, ch, look.bg));
                ops.push(vmap(vec![
                    ("op", s("text")),
                    ("x", fl(x)),
                    ("y", fl(y + text_dy)),
                    ("text", s(pe)),
                    ("size", fl(size)),
                    ("font", s(&font)),
                    ("color", rgb_val(look.fg)),
                ]));
                ops.push(rect(x, y + ch - 2.0, pw, 1.0, look.fg));
            } else if hollow {
                ops.push(vmap(vec![
                    ("op", s("rect")),
                    ("x", fl(x + 0.5)),
                    ("y", fl(y + 0.5)),
                    ("w", fl(w - 1.0)),
                    ("h", fl(ch - 1.0)),
                    ("stroke", rgb_val(look.cursor)),
                    ("width", fl(1.0)),
                ]));
            } else if style == "bar" {
                ops.push(rect(x, y, 2.0, ch, look.cursor));
            } else if style == "underline" {
                ops.push(rect(x, y + ch - 2.0, w, 2.0, look.cursor));
            } else {
                ops.push(rect(x, y, w, ch, look.cursor));
                let cell = line.cells.get(col).copied().unwrap_or(BLANK);
                if cell.ch != ' ' && cell.w != 0 {
                    let mut txt = cell.ch.to_string();
                    if let Some(x2) = line.extra_at(col) {
                        txt.push_str(x2);
                    }
                    let mut fields = vec![
                        ("op", s("text")),
                        ("x", fl(x)),
                        ("y", fl(y + text_dy)),
                        ("text", s(&txt)),
                        ("size", fl(size)),
                        ("font", s(&font)),
                        ("color", rgb_val(look.cursor_text)),
                    ];
                    if cell.a.fl & BOLD != 0 {
                        fields.push(("weight", Value::Float(bold_w)));
                    }
                    ops.push(vmap(fields));
                }
            }
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(t: &Term) -> Vec<String> {
        (0..t.rows).map(|r| t.line(r).text(0, t.cols).trim_end().to_string()).collect()
    }

    fn run(rows: usize, cols: usize, input: &str) -> Term {
        let mut t = Term::new(rows, cols, 100);
        t.feed(input.as_bytes());
        t
    }

    #[test]
    fn text_wraps_and_scrolls_into_the_scrollback() {
        let t = run(3, 5, "abcdefg\r\nxy\r\nz\r\nq");
        assert_eq!(screen(&t), vec!["xy", "z", "q"]);
        assert_eq!(t.lines[0].text(0, 5).trim_end(), "abcde");
        assert!(t.lines[0].wrapped);
        assert_eq!(t.lines[1].text(0, 5).trim_end(), "fg");
    }

    #[test]
    fn a_table_of_sequences() {
        let cases: Vec<(&str, Vec<&str>, (usize, usize))> = vec![
            ("hello", vec!["hello", "", ""], (0, 5)),
            ("ab\x1b[1;1Hx", vec!["xb", "", ""], (0, 1)),
            ("abc\x1b[2D\x1b[K", vec!["a", "", ""], (0, 1)),
            ("abc\r\ndef\x1b[2J", vec!["", "", ""], (1, 3)),
            ("abcdef\x1b[1;3H\x1b[2@", vec!["ab  cdef", "", ""], (0, 2)),
            ("abcdefghij\x1b[1;3H\x1b[2@", vec!["ab  cdefgh", "", ""], (0, 2)),
            ("abcdef\x1b[1;2H\x1b[2P", vec!["adef", "", ""], (0, 1)),
            ("abcdef\x1b[1;2H\x1b[3X", vec!["a   ef", "", ""], (0, 1)),
            ("1\r\n2\r\n3\x1b[1;1H\x1b[L", vec!["", "1", "2"], (0, 0)),
            ("1\r\n2\r\n3\x1b[1;1H\x1b[M", vec!["2", "3", ""], (0, 0)),
            ("a\tb", vec!["a       b", "", ""], (0, 9)),
            ("x\x1b[5bY", vec!["xxxxxxY", "", ""], (0, 7)),
            ("\x1b[2;1Hq\x1bMw", vec![" w", "q", ""], (0, 2)),
            ("\x1b(0lqk\x1b(B", vec!["┌─┐", "", ""], (0, 3)),
            ("ab\x1b7\x1b[3;3Hc\x1b8d", vec!["abd", "", "  c"], (0, 3)),
        ];
        for (input, want, cur) in cases {
            let t = run(3, 10, input);
            assert_eq!(screen(&t), want, "screen after {input:?}");
            assert_eq!((t.row, t.col), cur, "cursor after {input:?}");
        }
    }

    #[test]
    fn sgr_colours_and_flags() {
        let t = run(2, 20, "\x1b[1;3;4;31mA\x1b[0;38;5;200mB\x1b[38;2;1;2;3;48:2::9:8:7mC\x1b[7;9mD\x1b[22;23;24;27;29;39;49mE\x1b[92;104mF");
        let l = t.line(0);
        assert_eq!(l.cells[0].a.fl, BOLD | ITALIC | UNDERLINE);
        assert_eq!(l.cells[0].a.fg, IDX | 1);
        assert_eq!(l.cells[1].a.fg, IDX | 200);
        assert_eq!(l.cells[1].a.fl, 0);
        assert_eq!(l.cells[2].a.fg, RGB | 0x010203);
        assert_eq!(l.cells[2].a.bg, RGB | 0x090807);
        assert_eq!(l.cells[3].a.fl, INVERSE | STRIKE);
        assert_eq!(l.cells[4].a, Attr::default());
        assert_eq!(l.cells[5].a.fg, IDX | 10);
        assert_eq!(l.cells[5].a.bg, IDX | 12);
    }

    #[test]
    fn the_alternate_screen_keeps_the_main_one() {
        let mut t = run(3, 10, "main\r\nprompt$ ");
        t.feed(b"\x1b[?1049h\x1b[Hfull screen");
        assert!(t.alt.is_some());
        assert_eq!(screen(&t), vec!["full scree", "n", ""]);
        t.feed(b"\x1b[?1049l");
        assert!(t.alt.is_none());
        assert_eq!(screen(&t), vec!["main", "prompt$", ""]);
        assert_eq!((t.row, t.col), (1, 8));
    }

    #[test]
    fn a_scroll_region_scrolls_only_its_lines() {
        let mut t = run(4, 10, "a\r\nb\r\nc\r\nd");
        t.feed(b"\x1b[2;3r\x1b[3;1H\n\nX");
        assert_eq!(screen(&t), vec!["a", "", "X", "d"]);
        assert_eq!(t.lines.len(), 4, "nothing went into the scrollback");
        t.feed(b"\x1b[r\x1b[2;1H\x1b[2T");
        assert_eq!(screen(&t), vec!["", "", "a", ""]);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut t = run(2, 6, "a漢b");
        assert_eq!(t.line(0).cells[1].w, 2);
        assert_eq!(t.line(0).cells[2].w, 0);
        assert_eq!(t.col, 4);
        t.feed("\x1b[1;3Hx".as_bytes());
        // the wide character's second half overwritten: it is gone
        assert_eq!(screen(&t)[0], "a xb");
        let mut t = run(2, 4, "abc漢");
        assert_eq!(screen(&t), vec!["abc", "漢"]);
        t.feed("e\u{301}".as_bytes());
        assert_eq!(t.line(1).text(0, 4), "漢e\u{301} ");
    }

    #[test]
    fn answers_status_and_attributes() {
        let mut t = run(5, 10, "\x1b[3;4H\x1b[6n\x1b[c\x1b[?2004h\x1b[?2004$p\x1b]11;?\x07");
        let r = std::mem::take(&mut t.reply);
        assert!(r.starts_with("\x1b[3;4R\x1b[?62;22c\x1b[?2004;1$y\x1b]11;rgb:"), "{r:?}");
        assert!(t.bracketed);
    }

    #[test]
    fn titles_cwd_and_links() {
        let t = run(2, 20, "\x1b]2;my title\x07\x1b]7;file://host/Users/me/a%20b\x1b\\\x1b]8;;https://x.y/\x1b\\link\x1b]8;;\x1b\\ no");
        assert_eq!(t.title, "my title");
        assert_eq!(t.cwd, "/Users/me/a b");
        assert_eq!(t.line(0).cells[0].a.link, 1);
        assert_eq!(t.line(0).cells[3].a.link, 1);
        assert_eq!(t.line(0).cells[5].a.link, 0);
        assert_eq!(t.links[0], "https://x.y/");
    }

    #[test]
    fn resize_rewraps_the_main_screen() {
        let mut t = run(3, 10, "0123456789abcdef\r\nxy");
        assert_eq!(screen(&t), vec!["0123456789", "abcdef", "xy"]);
        t.resize(3, 20);
        assert_eq!(screen(&t), vec!["0123456789abcdef", "xy", ""]);
        assert_eq!((t.row, t.col), (1, 2));
        t.resize(3, 4);
        assert_eq!(screen(&t), vec!["89ab", "cdef", "xy"]);
        assert_eq!(t.lines.len(), 5);
        assert_eq!((t.row, t.col), (2, 2));
        t.resize(3, 10);
        assert_eq!(screen(&t), vec!["0123456789", "abcdef", "xy"]);
    }

    #[test]
    fn selection_text_and_find() {
        let t = run(3, 10, "hello world\r\nfoo bar\r\n");
        let s0 = t.first_id();
        assert_eq!(text_of(&t, (s0, 6), (s0 + 2, 3)), "world\nfoo");
        assert_eq!(find(&t, "O", false).len(), 4);
        assert_eq!(find(&t, "O", true).len(), 0);
    }

    #[test]
    fn find_crosses_a_wrap_and_keeps_the_newest() {
        // "hello worl" wraps to "d": "world" is one match across it
        let t = run(3, 10, "hello world\r\n");
        let s0 = t.first_id();
        let hits = find(&t, "world", false);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], tuple(vec![int(s0), int(6), int(5)]));
        // more than 2000 matches: the newest 2000, oldest first
        let mut many = String::new();
        for i in 0..2500 {
            many.push_str(&format!("x{i}\r\n"));
        }
        let mut t2 = Term::new(5, 20, 5000);
        t2.feed(many.as_bytes());
        let hits = find(&t2, "x", true);
        assert_eq!(hits.len(), 2000);
        let last_line = |v: &Value| match v {
            Value::Tuple(xs) => match &xs[0] {
                Value::Integer(i) => *i,
                _ => -1,
            },
            _ => -1,
        };
        assert!(last_line(&hits[0]) < last_line(&hits[1999]));
        let l = t2.line_by_id(last_line(&hits[1999])).unwrap();
        assert_eq!(l.text(0, 6).trim_end(), "x2499");
    }

    #[test]
    fn utf8_split_across_feeds() {
        let mut t = Term::new(2, 10, 10);
        let b = "é漢".as_bytes();
        t.feed(&b[..1]);
        t.feed(&b[1..4]);
        t.feed(&b[4..]);
        assert_eq!(screen(&t)[0], "é漢");
    }

    #[test]
    fn the_scrollback_is_bounded() {
        let mut t = Term::new(2, 10, 5);
        for i in 0..20 {
            t.feed(format!("{i}\r\n").as_bytes());
        }
        assert_eq!(t.lines.len(), 7);
        assert_eq!(t.first_id(), 14);
        assert_eq!(t.line_by_id(14).unwrap().text(0, 2).trim_end(), "14");
    }
}
