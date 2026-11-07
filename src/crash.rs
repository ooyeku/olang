//! Crash reports (OL-446): when a program asks (`runtime.on_crash(dir)`,
//! or `OLANG_CRASH_DIR` at start), a Rust panic that ends the process and
//! a fatal signal (SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP, SIGABRT) each
//! leave one JSON file in that folder before the process goes:
//!
//! ```json
//! { "format": 1, "kind": "crash", "reason": "panic" | "signal",
//!   "signal": "SIGSEGV", "message": "…", "location": "src/x.rs:1:2",
//!   "thread": { "name": "main", "id": 1234 },
//!   "olang": { "version": "1.0.0", "commit": "…" },
//!   "os": { "name": "macos", "version": "15.1" }, "arch": "aarch64",
//!   "time": 1730000000, "pid": 4321,
//!   "threads": [ { "name": "main", "id": 1234, "crashed": true,
//!                  "frames": [ { "function": "f", "file": "a.ol",
//!                                "line": 3, "tier": "vm" } ] } ],
//!   "native": { "frames": ["0x1000…"],
//!               "images": [ { "name": "…/olang", "uuid": "…",
//!                             "load_address": "0x1000…" } ] } }
//! ```
//!
//! What a report holds was decided with the owner: the panic's message
//! and Rust location, the thread that crashed, every thread's olang
//! frames (function, file, and for a panic the function's line), the
//! crashing thread's native backtrace as addresses with each image's
//! UUID and load address (`atos -o olang.dSYM -l <load_address> <addr>`
//! symbolicates them), the olang version and commit, the macOS version,
//! the CPU and the time. Never a value, the environment, the arguments
//! or a file's contents.
//!
//! A panic's report is written by the panic hook, where allocating is
//! fine. A signal's is written by a handler that may not lock or
//! allocate: it formats into a buffer set aside when the report was
//! armed, reads the shadow stacks through `profile`'s crash tables, and
//! writes the file with `open` and `write` alone.

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

/// Everything a handler needs, made when the report is armed.
struct Armed {
    /// `<dir>/olang-crash-<pid>-`, NUL-free; the handler appends the time.
    prefix: Vec<u8>,
    max_bytes: usize,
    /// The buffer a signal's report is formatted into.
    buf: *mut u8,
    os_version: String,
    images: Vec<Image>,
}

/// A loaded Mach-O image: where its text is and its UUID.
struct Image {
    name: String,
    start: usize,
    end: usize,
    uuid: String,
}

static ARMED: AtomicPtr<Armed> = AtomicPtr::new(std::ptr::null_mut());
/// Set by whichever report is written first: one crash, one report (a
/// panic's abort raises SIGABRT, which then writes nothing).
static REPORTED: AtomicBool = AtomicBool::new(false);

const SIGNALS: [libc::c_int; 6] = [
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGILL,
    libc::SIGFPE,
    libc::SIGTRAP,
    libc::SIGABRT,
];

/// The most a report may be (`max_bytes` is capped here).
pub const MAX_BYTES: usize = 1 << 20;
/// `max_bytes` when the program names none.
pub const DEFAULT_BYTES: usize = 256 * 1024;

/// Arm crash reports into `dir` (made if missing), each at most
/// `max_bytes`. Arming again moves them to the new folder.
pub fn install(dir: &std::path::Path, max_bytes: usize) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut prefix = dir
        .join(format!("olang-crash-{}-", std::process::id()))
        .to_string_lossy()
        .into_owned()
        .into_bytes();
    prefix.retain(|b| *b != 0);
    let max_bytes = max_bytes.clamp(16 * 1024, MAX_BYTES);
    let buf = Box::leak(vec![0u8; max_bytes].into_boxed_slice()).as_mut_ptr();
    let armed = Box::new(Armed {
        prefix,
        max_bytes,
        buf,
        os_version: os_version(),
        images: images(),
    });
    // the old one is leaked: a handler on another thread may be reading it
    ARMED.store(Box::into_raw(armed), Ordering::Release);
    crate::profile::crash_enable();
    install_handlers();
    Ok(dir)
}

/// Arm from `OLANG_CRASH_DIR`, when it is set (at start).
pub fn install_from_env() {
    if let Some(dir) = std::env::var_os("OLANG_CRASH_DIR").filter(|d| !d.is_empty()) {
        let _ = install(std::path::Path::new(&dir), DEFAULT_BYTES);
    }
}

pub fn armed() -> bool {
    !ARMED.load(Ordering::Acquire).is_null()
}

fn install_handlers() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for sig in SIGNALS {
            // SAFETY: a handler that only does async-signal-safe work, on
            // the thread's alternate stack (a stack overflow has no room).
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(sig, &action, std::ptr::null_mut());
            }
        }
    });
}

/// The fatal signal's handler: write the report (once), say where on
/// stderr, then end the process by the signal with its default action.
/// A fault and a signal someone sent cannot be told apart here (macOS
/// gives a `kill`ed SIGSEGV a fault's code), so neither goes back to the
/// previous handler: Rust's stack-overflow check would let a sent SIGSEGV
/// pass, and what it would say is in the report.
extern "C" fn on_signal(sig: libc::c_int, _info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    if !REPORTED.swap(true, Ordering::AcqRel) {
        let armed = ARMED.load(Ordering::Acquire);
        if !armed.is_null() {
            // SAFETY: armed is never freed; the buffer is ours alone, as
            // REPORTED lets one report through.
            unsafe { write_signal_report(&*armed, sig) };
        }
    }
    // SAFETY: sigaction, kill and getpid are async-signal-safe.
    unsafe {
        let mut dfl: libc::sigaction = std::mem::zeroed();
        dfl.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut dfl.sa_mask);
        libc::sigaction(sig, &dfl, std::ptr::null_mut());
        // to the process, not this thread (which blocks it while the
        // handler runs); a fault also runs again into the default action
        libc::kill(libc::getpid(), sig);
    }
}

fn signal_name(sig: libc::c_int) -> &'static str {
    match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGBUS => "SIGBUS",
        libc::SIGILL => "SIGILL",
        libc::SIGFPE => "SIGFPE",
        libc::SIGTRAP => "SIGTRAP",
        libc::SIGABRT => "SIGABRT",
        _ => "signal",
    }
}

// ── a writer that never allocates ────────────────────────────────────

/// Formats into a fixed buffer; what does not fit is dropped (the report
/// is closed properly by `finish`, which reserves its room).
struct Buf<'a> {
    bytes: &'a mut [u8],
    len: usize,
    /// Room kept for the closing brackets.
    reserve: usize,
    full: bool,
}

impl<'a> Buf<'a> {
    /// Where the next element starts.
    fn mark(&self) -> usize {
        self.len
    }
    /// Keep what was written since `mark` if it all fit; else take it
    /// back (false).
    fn keep(&mut self, mark: usize) -> bool {
        if self.full {
            self.len = mark;
            self.full = false;
            return false;
        }
        true
    }
    /// Closing punctuation, written into the room kept for it.
    fn close(&mut self, s: &str) {
        let s = s.as_bytes();
        if self.len + s.len() <= self.bytes.len() {
            self.bytes[self.len..self.len + s.len()].copy_from_slice(s);
            self.len += s.len();
        }
    }
    fn raw(&mut self, s: &[u8]) {
        let room = self.bytes.len().saturating_sub(self.reserve);
        if self.full || self.len + s.len() > room {
            self.full = true;
            return;
        }
        self.bytes[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }
    fn s(&mut self, s: &str) {
        self.raw(s.as_bytes());
    }
    /// A JSON string, escaped.
    fn json(&mut self, s: &str) {
        self.raw(b"\"");
        for &b in s.as_bytes() {
            match b {
                b'"' => self.raw(b"\\\""),
                b'\\' => self.raw(b"\\\\"),
                b'\n' => self.raw(b"\\n"),
                b'\r' => self.raw(b"\\r"),
                b'\t' => self.raw(b"\\t"),
                0..=0x1f => {
                    let hex = b"0123456789abcdef";
                    self.raw(&[
                        b'\\',
                        b'u',
                        b'0',
                        b'0',
                        hex[(b >> 4) as usize],
                        hex[(b & 15) as usize],
                    ]);
                }
                _ => self.raw(&[b]),
            }
        }
        self.raw(b"\"");
    }
    fn dec(&mut self, mut n: u64) {
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        loop {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.raw(&tmp[i..]);
    }
    fn hex(&mut self, n: usize) {
        let digits = b"0123456789abcdef";
        let mut tmp = [0u8; 18];
        tmp[0] = b'0';
        tmp[1] = b'x';
        let mut i = tmp.len();
        let mut n = n;
        loop {
            i -= 1;
            tmp[i] = digits[n & 15];
            n >>= 4;
            if n == 0 {
                break;
            }
        }
        self.raw(b"\"");
        self.raw(&tmp[..2]);
        self.raw(&tmp[i..]);
        self.raw(b"\"");
    }
}

unsafe extern "C" {
    fn backtrace(buffer: *mut *mut libc::c_void, size: libc::c_int) -> libc::c_int;
}

/// The parts every report shares, after the header: who, where, and the
/// stacks. Each list element is written whole or not at all, so a report
/// cut at `max_bytes` is still JSON (with `"truncated": true`).
fn common(b: &mut Buf<'_>, armed: &Armed, crashed: u64, time: u64) {
    b.s(",\"olang\":{\"version\":");
    b.json(crate::version::VERSION);
    b.s(",\"commit\":");
    b.json(crate::version::BUILD_COMMIT);
    b.s("},\"os\":{\"name\":");
    b.json(std::env::consts::OS);
    b.s(",\"version\":");
    b.json(&armed.os_version);
    b.s("},\"arch\":");
    b.json(std::env::consts::ARCH);
    b.s(",\"time\":");
    b.dec(time);
    b.s(",\"pid\":");
    // SAFETY: getpid is async-signal-safe.
    b.dec(unsafe { libc::getpid() } as u64);
    let mut truncated = false;

    // the native backtrace of the thread that crashed
    let mut addrs = [std::ptr::null_mut::<libc::c_void>(); 128];
    // SAFETY: a local array of the size given.
    let n = unsafe { backtrace(addrs.as_mut_ptr(), addrs.len() as libc::c_int) }.max(0) as usize;
    b.s(",\"native\":{\"frames\":[");
    let mut used = [false; 1024];
    for (i, a) in addrs[..n].iter().enumerate() {
        let a = *a as usize;
        let mark = b.mark();
        if i > 0 {
            b.s(",");
        }
        b.hex(a);
        if !b.keep(mark) {
            truncated = true;
            break;
        }
        if let Some(k) = armed
            .images
            .iter()
            .position(|im| a >= im.start && a < im.end)
            && k < used.len()
        {
            used[k] = true;
        }
    }
    b.close("],\"images\":[");
    let mut first_image = true;
    for (k, im) in armed.images.iter().enumerate() {
        // the images the backtrace passes through, and the main one
        if !(k == 0 || used.get(k).copied().unwrap_or(false)) {
            continue;
        }
        let mark = b.mark();
        if !first_image {
            b.s(",");
        }
        b.s("{\"name\":");
        b.json(&im.name);
        b.s(",\"uuid\":");
        b.json(&im.uuid);
        b.s(",\"load_address\":");
        b.hex(im.start);
        b.s("}");
        if !b.keep(mark) {
            truncated = true;
            break;
        }
        first_image = false;
    }
    b.close("]}");

    // every thread's olang frames, innermost first
    b.close(",\"threads\":[");
    let mut first = true;
    // SAFETY: the process is ending; racy reads of the stacks are fine.
    unsafe {
        crate::profile::crash_stacks(|t| {
            if truncated {
                return;
            }
            let mark = b.mark();
            if !first {
                b.s(",");
            }
            b.s("{\"name\":");
            match t.name {
                Some(n) => b.json(n),
                None => b.s("null"),
            }
            b.s(",\"id\":");
            b.dec(t.id);
            b.s(",\"crashed\":");
            b.s(if t.id == crashed { "true" } else { "false" });
            b.s(",\"depth\":");
            b.dec(t.depth as u64);
            b.s(",\"frames\":[");
            if !b.keep(mark) {
                truncated = true;
                return;
            }
            first = false;
            let mut first_frame = true;
            let mut last: Option<(&str, Option<&str>)> = None;
            for f in t.frames.iter().rev().flatten() {
                // a call promoted from the interpreter to the VM and to
                // native code is one frame, not three
                if last == Some((f.name, f.file)) {
                    continue;
                }
                last = Some((f.name, f.file));
                let mark = b.mark();
                if !first_frame {
                    b.s(",");
                }
                b.s("{\"function\":");
                b.json(f.name);
                b.s(",\"file\":");
                match f.file {
                    Some(file) => b.json(file),
                    None => b.s("null"),
                }
                b.s(",\"tier\":");
                b.json(f.tier);
                b.s("}");
                if !b.keep(mark) {
                    truncated = true;
                    break;
                }
                first_frame = false;
            }
            b.close("]}");
        });
    }
    b.close("]");
    if truncated {
        b.close(",\"truncated\":true");
    }
}

fn finish(b: &mut Buf<'_>) {
    b.close("}\n");
}

/// Write the report to `<prefix><time>.json` with open and write alone.
fn write_file(armed: &Armed, time: u64, bytes: &[u8]) -> bool {
    let mut path = [0u8; 4096];
    let mut n = 0;
    let mut push = |s: &[u8], n: &mut usize| {
        for &c in s {
            if *n + 1 < path.len() {
                path[*n] = c;
                *n += 1;
            }
        }
    };
    push(&armed.prefix, &mut n);
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    let mut t = time;
    loop {
        i -= 1;
        tmp[i] = b'0' + (t % 10) as u8;
        t /= 10;
        if t == 0 {
            break;
        }
    }
    push(&tmp[i..], &mut n);
    push(b".json", &mut n);
    path[n] = 0;
    // SAFETY: a NUL-terminated path in a local buffer; open/write/close
    // are async-signal-safe.
    unsafe {
        let fd = libc::open(
            path.as_ptr() as *const libc::c_char,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o600,
        );
        if fd < 0 {
            return false;
        }
        let mut off = 0;
        while off < bytes.len() {
            let w = libc::write(
                fd,
                bytes[off..].as_ptr() as *const libc::c_void,
                bytes.len() - off,
            );
            if w <= 0 {
                break;
            }
            off += w as usize;
        }
        libc::close(fd);
        off == bytes.len()
    }
}

/// # Safety
/// From the signal handler, once (REPORTED let it through).
unsafe fn write_signal_report(armed: &Armed, sig: libc::c_int) {
    // SAFETY: the buffer armed set aside, used by this one report.
    let bytes = unsafe { std::slice::from_raw_parts_mut(armed.buf, armed.max_bytes) };
    let mut b = Buf {
        bytes,
        len: 0,
        reserve: 96,
        full: false,
    };
    // SAFETY: time(NULL) is async-signal-safe.
    let time = unsafe { libc::time(std::ptr::null_mut()) }.max(0) as u64;
    let me = crate::profile::crash_current_thread();
    b.s("{\"format\":1,\"kind\":\"crash\",\"reason\":\"signal\",\"signal\":");
    b.json(signal_name(sig));
    b.s(",\"message\":");
    b.json(match sig {
        libc::SIGSEGV | libc::SIGBUS => {
            "the process touched memory it may not (a stack overflow, or a fault in native code)"
        }
        libc::SIGILL => "an illegal instruction",
        libc::SIGFPE => "an arithmetic fault",
        libc::SIGTRAP => "a trap (a Rust or system check that stops the process)",
        _ => "the process aborted",
    });
    b.s(",\"location\":null,\"thread\":{\"name\":null,\"id\":");
    b.dec(me);
    b.s("}");
    common(&mut b, armed, me, time);
    finish(&mut b);
    let len = b.len;
    // SAFETY: the formatted prefix of the buffer.
    let out = unsafe { std::slice::from_raw_parts(armed.buf, len) };
    if write_file(armed, time, out) {
        let say = |m: &[u8]| {
            // SAFETY: write(2) is async-signal-safe.
            unsafe { libc::write(2, m.as_ptr() as *const libc::c_void, m.len()) };
        };
        say(b"olang: ");
        say(signal_name(sig).as_bytes());
        say(b"; a crash report was written to ");
        say(&armed.prefix);
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        let mut t = time;
        loop {
            i -= 1;
            tmp[i] = b'0' + (t % 10) as u8;
            t /= 10;
            if t == 0 {
                break;
            }
        }
        say(&tmp[i..]);
        say(b".json\n");
    }
}

/// Called by the panic hook for a panic that will end the process: the
/// report, when one is armed. Answers the file written.
pub fn on_panic(
    message: &str,
    location: Option<String>,
    thread: &str,
) -> Option<std::path::PathBuf> {
    let armed = ARMED.load(Ordering::Acquire);
    if armed.is_null() || REPORTED.swap(true, Ordering::AcqRel) {
        return None;
    }
    // SAFETY: armed is never freed.
    let armed = unsafe { &*armed };
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let me = crate::profile::crash_current_thread();
    let mut bytes = vec![0u8; armed.max_bytes];
    let mut b = Buf {
        bytes: &mut bytes,
        len: 0,
        reserve: 96,
        full: false,
    };
    b.s("{\"format\":1,\"kind\":\"crash\",\"reason\":\"panic\",\"signal\":null,\"message\":");
    // a message long enough to crowd out the stacks is cut
    let message = match message.char_indices().nth(4096) {
        Some((at, _)) => &message[..at],
        None => message,
    };
    b.json(message);
    b.s(",\"location\":");
    match &location {
        Some(l) => b.json(l),
        None => b.s("null"),
    }
    b.s(",\"thread\":{\"name\":");
    b.json(thread);
    b.s(",\"id\":");
    b.dec(me);
    b.s("}");
    common(&mut b, armed, me, time);
    finish(&mut b);
    let len = b.len;
    bytes.truncate(len);
    // A panic may allocate: give each olang frame its function's line.
    let bytes = with_lines(bytes);
    let path = {
        let mut p = armed.prefix.clone();
        p.extend_from_slice(format!("{time}.json").as_bytes());
        std::path::PathBuf::from(String::from_utf8_lossy(&p).into_owned())
    };
    write_file(armed, time, &bytes).then_some(path)
}

/// The report with `"line"` added to each olang frame whose file says
/// where its function is declared (a panic's report only).
fn with_lines(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return bytes;
    };
    let mut lines = crate::profile::LineCache::default();
    if let Some(threads) = v.get_mut("threads").and_then(|t| t.as_array_mut()) {
        for t in threads {
            if let Some(frames) = t.get_mut("frames").and_then(|f| f.as_array_mut()) {
                for f in frames {
                    let line = match (
                        f.get("function").and_then(|x| x.as_str()),
                        f.get("file").and_then(|x| x.as_str()),
                    ) {
                        (Some(name), Some(file)) => lines.line_of(name, file),
                        _ => None,
                    };
                    if let Some(obj) = f.as_object_mut() {
                        obj.insert(
                            "line".to_string(),
                            line.map(serde_json::Value::from)
                                .unwrap_or(serde_json::Value::Null),
                        );
                    }
                }
            }
        }
    }
    let mut out = serde_json::to_vec(&v).unwrap_or(bytes);
    out.push(b'\n');
    out
}

/// The macOS version (`kern.osproductversion`).
fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 64];
        let mut len = buf.len();
        // SAFETY: a sysctl read into a local buffer of the size given.
        let ok = unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        if ok {
            let end = buf[..len].iter().position(|b| *b == 0).unwrap_or(len);
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    String::new()
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_header(i: u32) -> *const u8;
    fn _dyld_get_image_name(i: u32) -> *const libc::c_char;
}

/// The images loaded now, the main executable first: each one's text and
/// UUID, read from its Mach-O header.
fn images() -> Vec<Image> {
    #[cfg(target_os = "macos")]
    {
        let mut out = Vec::new();
        // SAFETY: dyld's own tables, read while the images stay loaded;
        // each header is a live mach_header_64 followed by its commands.
        unsafe {
            for i in 0.._dyld_image_count() {
                let h = _dyld_get_image_header(i);
                if h.is_null() {
                    continue;
                }
                let name = _dyld_get_image_name(i);
                let name = if name.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(name)
                        .to_string_lossy()
                        .into_owned()
                };
                let rd32 = |p: *const u8| std::ptr::read_unaligned(p as *const u32);
                let rd64 = |p: *const u8| std::ptr::read_unaligned(p as *const u64);
                if rd32(h) != 0xfeed_facf {
                    continue;
                }
                let ncmds = rd32(h.add(16));
                let mut at = h.add(32);
                let mut uuid = String::new();
                let mut text = 0usize;
                for _ in 0..ncmds {
                    let cmd = rd32(at);
                    let size = rd32(at.add(4)) as usize;
                    if cmd == 0x1b {
                        let bytes = std::slice::from_raw_parts(at.add(8), 16);
                        uuid = bytes.iter().map(|b| format!("{b:02X}")).collect();
                        uuid.insert(20, '-');
                        uuid.insert(16, '-');
                        uuid.insert(12, '-');
                        uuid.insert(8, '-');
                    } else if cmd == 0x19 {
                        let seg = std::slice::from_raw_parts(at.add(8), 16);
                        if seg.starts_with(b"__TEXT\0") {
                            text = rd64(at.add(32)) as usize;
                        }
                    }
                    if size == 0 {
                        break;
                    }
                    at = at.add(size);
                }
                out.push(Image {
                    name,
                    start: h as usize,
                    end: h as usize + text,
                    uuid,
                });
            }
        }
        out
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

// The Armed pointer and its buffer are shared with signal handlers.
unsafe impl Send for Armed {}
unsafe impl Sync for Armed {}
