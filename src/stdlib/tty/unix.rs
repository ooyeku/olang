//! The `tty` module on Unix: termios, the window size, the wake pipe the
//! signal actions write to, and the reader thread that decodes stdin.

use super::decode::{Decoder, Input};
use super::guard;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

/// Write all of `bytes` to `fd`, retrying on EINTR. Async-signal-safe.
pub(crate) fn write_fd(fd: libc::c_int, bytes: &[u8]) -> bool {
    let mut off = 0usize;
    while off < bytes.len() {
        // SAFETY: a write of a live slice.
        let n = unsafe {
            libc::write(
                fd,
                bytes[off..].as_ptr() as *const libc::c_void,
                bytes.len() - off,
            )
        };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return false;
        }
        if n == 0 {
            return false;
        }
        off += n as usize;
    }
    true
}

pub(crate) fn is_tty(fd: libc::c_int) -> bool {
    // SAFETY: isatty on a descriptor number.
    unsafe { libc::isatty(fd) == 1 }
}

/// The terminal's size, `(cols, rows)`, from stdout (or stdin).
pub(crate) fn size() -> Result<(u16, u16), String> {
    for fd in [libc::STDOUT_FILENO, libc::STDIN_FILENO] {
        if !is_tty(fd) {
            continue;
        }
        // SAFETY: TIOCGWINSZ fills a winsize.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };
        if r == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            return Ok((ws.ws_col, ws.ws_row));
        }
    }
    Err("tty.size: stdout is not a terminal".to_string())
}

pub(crate) fn get_termios() -> Result<libc::termios, String> {
    // SAFETY: tcgetattr fills a termios.
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) } != 0 {
        return Err(format!(
            "tty.enter: cannot read the terminal's settings: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(t)
}

/// `t` in raw mode, as `cfmakeraw(3)` makes it: no echo, no line
/// editing, no signals from ctrl+c / ctrl+z / ctrl+\ (they arrive as
/// keys), no CR/NL translation on input or output, 8-bit characters.
pub(crate) fn raw_of(t: &libc::termios) -> libc::termios {
    let mut raw = *t;
    // SAFETY: cfmakeraw edits the struct in place.
    unsafe { libc::cfmakeraw(&mut raw) };
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    raw
}

pub(crate) fn set_termios(t: &libc::termios) -> Result<(), String> {
    // SAFETY: tcsetattr on stdin.
    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t) } != 0 {
        return Err(format!(
            "tty.enter: cannot change the terminal's settings: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

// ── the wake pipe ────────────────────────────────────────────────────

static WAKE_R: AtomicI32 = AtomicI32::new(-1);
static WAKE_W: AtomicI32 = AtomicI32::new(-1);

/// Create the wake pipe, once: a signal action writes a byte, the
/// reader's poll wakes. Both ends non-blocking and close-on-exec.
pub(crate) fn wake_pipe() -> std::io::Result<()> {
    if WAKE_R.load(Ordering::SeqCst) >= 0 {
        return Ok(());
    }
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe fills two descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for fd in fds {
        // SAFETY: fcntl on descriptors we own.
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    WAKE_R.store(fds[0], Ordering::SeqCst);
    WAKE_W.store(fds[1], Ordering::SeqCst);
    Ok(())
}

/// Wake the reader. Async-signal-safe; a full pipe already means "awake".
pub(crate) fn wake() {
    let w = WAKE_W.load(Ordering::SeqCst);
    if w >= 0 {
        let b = [1u8];
        // SAFETY: a non-blocking one-byte write.
        unsafe {
            libc::write(w, b.as_ptr() as *const libc::c_void, 1);
        }
    }
}

fn drain_wake(fd: libc::c_int) {
    let mut buf = [0u8; 64];
    // SAFETY: non-blocking reads into a stack buffer.
    while unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) } > 0 {}
}

// ── the reader ───────────────────────────────────────────────────────

/// Read stdin until told to stop: decode, route replies to a waiting
/// query, send everything else to the events channel, and turn the
/// signals the guard noted into events.
pub(crate) fn reader(ctx: super::ReaderCtx) {
    let _live = crate::stdlib::chan::live_guard();
    let wake_r = WAKE_R.load(Ordering::SeqCst);
    let mut decoder = Decoder::new();
    let mut pending_since: Option<Instant> = None;
    let mut buf = [0u8; 4096];
    loop {
        if ctx.stop.load(Ordering::SeqCst) {
            break;
        }
        let timeout: libc::c_int = match (decoder.pending(), pending_since) {
            (Some(ms), Some(since)) => {
                let waited = since.elapsed().as_millis() as u64;
                ms.saturating_sub(waited) as libc::c_int
            }
            _ => 250,
        };
        let mut fds = [
            libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_r,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let nfds = if wake_r >= 0 { 2 } else { 1 };
        // SAFETY: poll over a live array.
        let r = unsafe { libc::poll(fds.as_mut_ptr(), nfds, timeout) };
        if r < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if ctx.stop.load(Ordering::SeqCst) {
            break;
        }
        if nfds == 2 && fds[1].revents != 0 {
            drain_wake(wake_r);
        }
        let bits = guard::take_signals();
        if bits != 0 {
            ctx.signals(bits);
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            // SAFETY: a read into a stack buffer.
            let n = unsafe {
                libc::read(
                    libc::STDIN_FILENO,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n < 0 {
                let e = std::io::Error::last_os_error().raw_os_error();
                if e == Some(libc::EINTR) || e == Some(libc::EAGAIN) {
                    continue;
                }
                ctx.eof();
                break;
            }
            if n == 0 {
                ctx.eof();
                break;
            }
            decoder.expect_cpr = ctx.querying();
            let inputs = decoder.feed(&buf[..n as usize]);
            ctx.deliver(inputs);
            pending_since = decoder.pending().map(|_| Instant::now());
        } else if r == 0 && decoder.pending().is_some() {
            let inputs: Vec<Input> = decoder.flush();
            ctx.deliver(inputs);
            pending_since = None;
        }
    }
}
