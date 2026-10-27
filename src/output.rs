//! Where `print` and `println` go.
//!
//! Native builds write straight to stdout, exactly as before. The
//! playground build (no "native" feature) has no stdout — output collects
//! in a per-thread buffer that the playground entry point drains after
//! each run and hands back to the page.

#[cfg(not(feature = "native"))]
use std::cell::RefCell;

#[cfg(not(feature = "native"))]
thread_local! {
    static CAPTURE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Native-path write that never panics on a vanished reader. `print!`
/// and `println!` abort the whole process with "failed printing to
/// stdout" when the pipe closes — which is exactly what happens in
/// `olang gen.ol | head -1` the moment head exits. Emulate the Unix
/// SIGPIPE default instead: terminate quietly with the conventional
/// 141 (128+SIGPIPE), like every well-behaved filter. Other write
/// errors report once to stderr (if it is still there) and exit 1
/// (`crate::stdio::stdout_failed`).
#[cfg(feature = "native")]
fn write_stdout(bytes: &[u8], flush: bool) {
    use std::io::Write;
    if crate::stdio::capturing()
        && crate::stdio::capture_push(false, &String::from_utf8_lossy(bytes))
    {
        return;
    }
    if DISCARDING.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if COLLECTING.load(std::sync::atomic::Ordering::Relaxed) {
        if let Ok(mut buf) = COLLECTED.lock() {
            buf.push(bytes);
        }
        if TAPPED.load(std::sync::atomic::Ordering::Relaxed) {
            tap_push(bytes);
        }
        return;
    }
    let mut out = std::io::stdout().lock();
    let result = out
        .write_all(bytes)
        .and_then(|_| if flush { out.flush() } else { Ok(()) });
    if let Err(e) = result {
        drop(out);
        crate::stdio::stdout_failed(e);
    }
}

/// Write `text` with no trailing newline.
pub fn emit(text: &str) {
    #[cfg(feature = "native")]
    {
        // Flush so partial lines appear immediately — a shell prompt or
        // progress indicator printed with `print` must not sit in the
        // buffer waiting for a newline.
        write_stdout(text.as_bytes(), true);
    }
    #[cfg(not(feature = "native"))]
    CAPTURE.with(|c| c.borrow_mut().push_str(text));
}

/// Write `text` followed by a newline.
pub fn emit_line(text: &str) {
    #[cfg(feature = "native")]
    {
        let mut line = String::with_capacity(text.len() + 1);
        line.push_str(text);
        line.push('\n');
        write_stdout(line.as_bytes(), false);
    }
    #[cfg(not(feature = "native"))]
    CAPTURE.with(|c| {
        let mut buf = c.borrow_mut();
        buf.push_str(text);
        buf.push('\n');
    });
}

/// Write `text` to standard error, as it is (`eprint`, `eprintln`). A
/// stderr that has gone drops it. The playground has no stderr: the text
/// joins the output it collects.
pub fn emit_err(text: &str) {
    #[cfg(feature = "native")]
    crate::stdio::err(format_args!("{}", text));
    #[cfg(not(feature = "native"))]
    CAPTURE.with(|c| c.borrow_mut().push_str(text));
}

/// Take everything captured on this thread since the last drain.
#[cfg(not(feature = "native"))]
pub fn drain_captured() -> String {
    CAPTURE.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

// `olang repl --serve` keeps its stdout for the protocol: what a program
// prints while it serves is collected here, from any thread, and handed
// back with the evaluation that printed it.
#[cfg(feature = "native")]
static COLLECTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "native")]
static COLLECTED: std::sync::Mutex<Collected> = std::sync::Mutex::new(Collected {
    head: Vec::new(),
    tail: Vec::new(),
    dropped: 0,
});

/// What an evaluation printed, bounded: a loop printing millions of lines
/// made one reply of tens of megabytes (and held them all), which the
/// editor then had to read and lay out. Its first `HEAD_MAX` bytes and its
/// last `TAIL_MAX` are kept, and how much fell between them.
#[cfg(feature = "native")]
struct Collected {
    head: Vec<u8>,
    tail: Vec<u8>,
    dropped: u64,
}

#[cfg(feature = "native")]
const HEAD_MAX: usize = 512 * 1024;
#[cfg(feature = "native")]
const TAIL_MAX: usize = 256 * 1024;

#[cfg(feature = "native")]
impl Collected {
    fn push(&mut self, bytes: &[u8]) {
        let room = HEAD_MAX.saturating_sub(self.head.len());
        let (h, t) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(h);
        if t.is_empty() {
            return;
        }
        self.tail.extend_from_slice(t);
        // trimmed in steps, not on every write: a line at a time stays cheap
        if self.tail.len() > 2 * TAIL_MAX {
            let cut = self.tail.len() - TAIL_MAX;
            self.tail.drain(..cut);
            self.dropped += cut as u64;
        }
    }

    fn take(&mut self) -> String {
        let mut head = std::mem::take(&mut self.head);
        let mut tail = std::mem::take(&mut self.tail);
        let mut dropped = std::mem::take(&mut self.dropped);
        if tail.len() > TAIL_MAX {
            let cut = tail.len() - TAIL_MAX;
            tail.drain(..cut);
            dropped += cut as u64;
        }
        if dropped == 0 {
            head.extend_from_slice(&tail);
            return String::from_utf8_lossy(&head).into_owned();
        }
        format!(
            "{}\n… {} bytes of output not kept …\n{}",
            String::from_utf8_lossy(&head),
            dropped,
            String::from_utf8_lossy(&tail)
        )
    }
}

/// From now on `print` and `println` write nothing: `olang lsp`'s stdout
/// is its protocol, and nothing the server evaluates (a macro's
/// expansion) may write a message onto it.
#[cfg(feature = "native")]
pub fn discard_output() {
    DISCARDING.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "native")]
static DISCARDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From now on `print` and `println` collect instead of writing.
#[cfg(feature = "native")]
pub fn collect_output() {
    COLLECTING.store(true, std::sync::atomic::Ordering::SeqCst);
}

// While an evaluation runs under `repl --serve`, what it prints is also
// handed to a tap as it is printed (the protocol's `out` events), so an
// editor shows a long cell's lines as they come. The tap hears batches:
// at most one every `TAP_EVERY`, the rest sent by a ticker within
// `TAP_TICK`, and nothing past `HEAD_MAX` bytes (the reply carries the
// whole, bounded as above).
#[cfg(feature = "native")]
pub type Tap = Box<dyn Fn(&str) + Send>;

#[cfg(feature = "native")]
struct Tapping {
    send: Tap,
    pending: Vec<u8>,
    last: std::time::Instant,
    sent: usize,
}

#[cfg(feature = "native")]
static TAPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "native")]
static TAP: std::sync::Mutex<Option<Tapping>> = std::sync::Mutex::new(None);
#[cfg(feature = "native")]
const TAP_EVERY: std::time::Duration = std::time::Duration::from_millis(50);
#[cfg(feature = "native")]
const TAP_TICK: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(feature = "native")]
impl Tapping {
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        // a character split across writes waits for its other half
        let cut = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => self.pending.len(),
        };
        let room = HEAD_MAX.saturating_sub(self.sent);
        let text: Vec<u8> = self.pending.drain(..cut).collect();
        if room > 0 && !text.is_empty() {
            let text = &text[..text.len().min(room)];
            self.sent += text.len();
            (self.send)(&String::from_utf8_lossy(text));
        }
        self.last = std::time::Instant::now();
    }
}

#[cfg(feature = "native")]
fn tap_push(bytes: &[u8]) {
    if let Ok(mut tap) = TAP.lock() {
        if let Some(t) = tap.as_mut() {
            t.pending.extend_from_slice(bytes);
            if t.last.elapsed() >= TAP_EVERY {
                t.flush();
            }
        }
    }
}

/// Hand what is printed from now on to `send` too, in batches, until
/// `tap_end`. A ticker thread sends what waits.
#[cfg(feature = "native")]
pub fn tap_begin(send: Tap) {
    static TICKER: std::sync::Once = std::sync::Once::new();
    TICKER.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("olang-out-tap".into())
            .spawn(|| {
                loop {
                    std::thread::sleep(TAP_TICK);
                    if TAPPED.load(std::sync::atomic::Ordering::Relaxed) {
                        if let Ok(mut tap) = TAP.lock() {
                            if let Some(t) = tap.as_mut() {
                                t.flush();
                            }
                        }
                    }
                }
            });
    });
    if let Ok(mut tap) = TAP.lock() {
        *tap = Some(Tapping {
            send,
            pending: Vec::new(),
            last: std::time::Instant::now(),
            sent: 0,
        });
    }
    TAPPED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Send what waits and close the tap: nothing it hears comes after this.
#[cfg(feature = "native")]
pub fn tap_end() {
    TAPPED.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut tap) = TAP.lock() {
        if let Some(mut t) = tap.take() {
            t.flush();
            // a lone half of a character at the very end
            if !t.pending.is_empty() {
                let rest = std::mem::take(&mut t.pending);
                if t.sent < HEAD_MAX {
                    (t.send)(&String::from_utf8_lossy(&rest));
                }
            }
        }
    }
}

/// What was printed since the last call (lossy UTF-8).
#[cfg(feature = "native")]
pub fn take_collected() -> String {
    match COLLECTED.lock() {
        Ok(mut buf) => buf.take(),
        Err(_) => String::new(),
    }
}
