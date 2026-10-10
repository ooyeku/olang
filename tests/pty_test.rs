//! `pty` and `vt`: a child on a real pseudo-terminal (`/bin/echo`, an
//! interactive `/bin/cat`, `stty size` after a resize, a shell killed),
//! and a terminal's screen read and drawn from olang code. Unix only.
#![cfg(unix)]

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

fn assert_all_true(source: &str, n: usize) {
    let result = eval(source).unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); n].into()), "{source}");
}

// Every helper reads until `want` shows up in what the pty said (or 3 s).
const READ_UNTIL: &str = r#"
fn read_until(p, want) = {
    let mut seen = ""
    let mut going = true
    let mut n = 0
    while going && n < 60 {
        n = n + 1
        match pty.read(p, #{ "timeout_ms": 50 }) {
            Ok(b) => if b != () => { seen = seen + unwrap(bytes.to_string(b)) },
            Err(e) => { going = false }
        }
        if str.contains(seen, want) => { going = false }
    }
    seen
}
"#;

#[test]
fn echo_on_a_pty_says_its_words_and_ends() {
    assert_all_true(
        &format!(
            r#"{READ_UNTIL}
let p = unwrap(pty.spawn(["/bin/echo", "hello", "pty"], #{{ "rows": 10, "cols": 40 }}))
let out = read_until(p, "hello pty")
let ended = unwrap(pty.wait(p, 2000))
let eof = match pty.read(p, #{{ "timeout_ms": 200 }}) {{ Ok(b) => b == (), Err(e) => e == "eof" }}
pty.close(p)
[str.contains(out, "hello pty\r\n"), ended.code == 0, eof, is_err(pty.write(p, "x"))]
"#
        ),
        4,
    );
}

#[test]
fn cat_echoes_what_is_typed_and_answers_eof() {
    assert_all_true(
        &format!(
            r#"{READ_UNTIL}
let p = unwrap(pty.spawn(["/bin/cat"], #{{ "rows": 24, "cols": 80, "env": #{{ "TERM": "xterm-256color" }} }}))
unwrap(pty.write(p, "ping\r"))
let out = read_until(p, "ping\r\nping\r\n")
let fg = pty.foreground(p)
unwrap(pty.write(p, "\u{{4}}"))
let ended = unwrap(pty.wait(p, 2000))
pty.close(p)
[str.contains(out, "ping\r\nping\r\n"), fg.name == "cat", fg.shell, ended.code == 0]
"#
        ),
        4,
    );
}

#[test]
fn a_resize_reaches_stty_and_the_cwd_is_read() {
    assert_all_true(
        &format!(
            r#"{READ_UNTIL}
let p = unwrap(pty.spawn(["/bin/sh"], #{{ "rows": 24, "cols": 80, "cwd": "/tmp", "env": #{{ "PS1": "$ ", "ENV": () }} }}))
let a = read_until(p, "$ ")
unwrap(pty.write(p, "stty size\r"))
let b = read_until(p, "24 80")
unwrap(pty.resize(p, 33, 101))
unwrap(pty.write(p, "stty size\r"))
let c = read_until(p, "33 101")
let dir = pty.cwd(p)
unwrap(pty.write(p, "exit 3\r"))
// a terminal closing waits for what it wrote to be read (macOS)
let bye = read_until(p, "never")
let ended = unwrap(pty.wait(p, 2000))
pty.close(p)
[str.contains(b, "24 80"), str.contains(c, "33 101"), dir == "/tmp" || dir == "/private/tmp", ended.code == 3]
"#
        ),
        4,
    );
}

#[test]
fn kill_hangs_up_a_running_job_and_close_reaps_it() {
    assert_all_true(
        &format!(
            r#"{READ_UNTIL}
let p = unwrap(pty.spawn(["/bin/sh", "-c", "sleep 30"], #{{ "rows": 24, "cols": 80 }}))
let still = unwrap(pty.wait(p, 100))
unwrap(pty.kill(p))
let ended = unwrap(pty.wait(p, 3000))
pty.close(p)
let gone = is_err(pty.write(p, "x"))
[still == (), ended != (), ended.signal == 1 || ended.code != 0, gone]
"#
        ),
        4,
    );
}

#[test]
fn close_ends_a_shell_that_ignores_nothing() {
    // the shell's child keeps the terminal; close hangs both up
    let out = eval(
        r#"
let p = unwrap(pty.spawn(["/bin/sh", "-c", "sleep 30 & wait"], #{ "rows": 24, "cols": 80 }))
let pid = pty.pid(p)
pty.close(p)
pid
"#,
    )
    .unwrap();
    let Value::Integer(pid) = out else { panic!("{out:?}") };
    let mut alive = true;
    for _ in 0..100 {
        // SAFETY: signal 0 only asks whether the pid exists.
        if unsafe { libc::kill(pid as i32, 0) } != 0 {
            alive = false;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!alive, "the shell outlived pty.close");
}

#[test]
fn a_screen_reads_sequences_and_draws_operations() {
    assert_all_true(
        r##"
let t = vt.new(4, 20, #{ "scrollback": 100 })
let reply = vt.feed(t, "\u{1b}]2;title\u{7}\u{1b}[1;31mred\u{1b}[0m plain\r\n\u{1b}[6n")
let i = vt.info(t)
let ops = vt.render(t, #{ "cell_w": 8, "cell_h": 16, "size": 13, "palette": ["#000000", "#ff0000"] })
let texts = filter(ops, (o) => map_get(o, "op") == "text")
let red = filter(texts, (o) => map_get(o, "text") == "red")
let line = vt.line(t, i.screen)
let sel = vt.text(t, i.screen, 4, i.screen, 9)
let more = vt.feed(t, "\u{1b}[?1049h\u{1b}[?2004h")
let j = vt.info(t)
vt.free(t)
[reply == "\u{1b}[2;1R", i.title == "red" || i.title == "title", len(red) == 1, map_get(red[0], "weight") == 700.0,
 map_get(red[0], "color") == (255, 0, 0), line.text == "red plain", sel == "plain", j.alt, j.bracketed_paste, more == ""]
"##,
        10,
    );
}
