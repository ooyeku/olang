//! One reader for every child's output pipes (OL-497).
//!
//! A child spawned by `proc.spawn` (or run by `os.exec` with a deadline)
//! has its stdout and stderr drained as fast as it writes them, so it never
//! blocks on a full pipe while the program reads the other stream or does
//! something else. That took two threads a child. Here one thread, made
//! when the first pipe is watched and gone once the last one closes,
//! `poll(2)`s every pipe and hands what it reads to the pipe's [`Sink`]:
//! the same lines, frames, chunks and bounded stderr, in the same order,
//! buffered without bound as before, so back-pressure is what it was.
//!
//! Unix only; elsewhere [`watch`] gives each pipe a thread, as before.

use std::io::Read;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Where a pipe's bytes go.
pub(crate) enum Sink {
    /// a line at a time, its `\n` (and a `\r` before it) removed; a last
    /// line without one is sent at the end
    Lines(Sender<Vec<u8>>),
    /// the raw chunks, as they come
    Chunks(Sender<Vec<u8>>),
    /// messages framed by `Content-Length` headers, each body sent whole
    /// (as text: invalid UTF-8 replaced)
    Frames(Sender<Vec<u8>>),
    /// everything, its last `keep` bytes kept, stored at the end
    Bounded {
        into: Arc<Mutex<Vec<u8>>>,
        keep: usize,
    },
    /// everything, sent once at the end with the tag
    Whole {
        tx: Sender<(bool, Vec<u8>)>,
        tag: bool,
    },
}

/// A sink with what it holds of a stream so far.
struct Feed {
    sink: Sink,
    buf: Vec<u8>,
    /// Bounded: bytes dropped from the front
    dropped: u64,
    /// Frames: the body length the headers gave
    frame_len: Option<usize>,
    /// Frames: the headers are read, the body is next
    in_body: bool,
}

impl Feed {
    fn new(sink: Sink) -> Self {
        Feed {
            sink,
            buf: Vec::new(),
            dropped: 0,
            frame_len: None,
            in_body: false,
        }
    }

    /// Take `data`; `false` when the stream should be read no further
    /// (its reader gone, or a frame that cannot be read).
    fn push(&mut self, data: &[u8]) -> bool {
        match &self.sink {
            Sink::Chunks(tx) => tx.send(data.to_vec()).is_ok(),
            Sink::Lines(tx) => {
                self.buf.extend_from_slice(data);
                let mut start = 0;
                let mut alive = true;
                while let Some(i) = self.buf[start..].iter().position(|&b| b == b'\n') {
                    let end = start + i;
                    let mut line = self.buf[start..end].to_vec();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    start = end + 1;
                    if tx.send(line).is_err() {
                        alive = false;
                        break;
                    }
                }
                self.buf.drain(..start);
                alive
            }
            Sink::Frames(_) => {
                self.buf.extend_from_slice(data);
                self.frames()
            }
            Sink::Bounded { keep, .. } => {
                let keep = *keep;
                self.buf.extend_from_slice(data);
                if self.buf.len() > keep + keep / 4 {
                    let mut cut = self.buf.len() - keep;
                    // from the next line's start, when one is close
                    if let Some(i) = self.buf[cut..].iter().take(4096).position(|&b| b == b'\n') {
                        cut += i + 1;
                    }
                    self.buf.drain(..cut);
                    self.dropped += cut as u64;
                }
                true
            }
            Sink::Whole { .. } => {
                self.buf.extend_from_slice(data);
                true
            }
        }
    }

    /// Send every whole frame in `buf`, as `read_frame` read them: header
    /// lines until a blank one after a `Content-Length`, then the body.
    fn frames(&mut self) -> bool {
        let Sink::Frames(tx) = &self.sink else {
            return true;
        };
        loop {
            if self.in_body {
                let Some(len) = self.frame_len else {
                    return false;
                };
                if self.buf.len() < len {
                    return true;
                }
                let body: Vec<u8> = self.buf.drain(..len).collect();
                self.in_body = false;
                self.frame_len = None;
                let text = String::from_utf8_lossy(&body).into_owned();
                if tx.send(text.into_bytes()).is_err() {
                    return false;
                }
                continue;
            }
            let Some(i) = self.buf.iter().position(|&b| b == b'\n') else {
                return true;
            };
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            // a header line that is not text ends the stream, as
            // `read_line` into a String did
            let Ok(line) = std::str::from_utf8(&line) else {
                return false;
            };
            let t = line.trim_end_matches(['\r', '\n']);
            if t.is_empty() {
                if self.frame_len.is_some() {
                    self.in_body = true;
                }
                continue;
            }
            if let Some((k, v)) = t.split_once(':')
                && k.trim().eq_ignore_ascii_case("content-length")
            {
                self.frame_len = v.trim().parse().ok();
            }
        }
    }

    /// The stream ended: `clean` at its end of file, not on an error.
    fn finish(mut self, clean: bool) {
        match self.sink {
            Sink::Lines(tx) => {
                if clean && !self.buf.is_empty() {
                    let mut line = std::mem::take(&mut self.buf);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    let _ = tx.send(line);
                }
            }
            Sink::Chunks(_) | Sink::Frames(_) => {}
            Sink::Bounded { into, keep } => {
                let mut buf = self.buf;
                if buf.len() > keep {
                    let cut = buf.len() - keep;
                    buf.drain(..cut);
                    self.dropped += cut as u64;
                }
                let out = if self.dropped == 0 {
                    buf
                } else {
                    let mut out =
                        format!("[… {} bytes of stderr dropped …]\n", self.dropped).into_bytes();
                    out.extend_from_slice(&buf);
                    out
                };
                *into.lock().unwrap_or_else(|e| e.into_inner()) = out;
            }
            Sink::Whole { tx, tag } => {
                let _ = tx.send((tag, self.buf));
            }
        }
    }
}

/// Drain `pipe` into `sink` until its end.
#[cfg(unix)]
pub(crate) fn watch(pipe: impl Into<std::os::fd::OwnedFd>, sink: Sink) {
    unix::watch(pipe.into(), Feed::new(sink));
}

/// Drain `pipe` into `sink` until its end, on a thread of its own.
#[cfg(not(unix))]
pub(crate) fn watch(pipe: impl Read + Send + 'static, sink: Sink) {
    std::thread::spawn(move || drain(pipe, sink));
}

/// Read `pipe` to its end into `sink`, on this thread.
#[allow(dead_code)]
pub(crate) fn drain(mut pipe: impl Read, sink: Sink) {
    let mut feed = Feed::new(sink);
    let mut chunk = vec![0u8; 65536];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => return feed.finish(true),
            Ok(n) => {
                if !feed.push(&chunk[..n]) {
                    return feed.finish(false);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return feed.finish(false),
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::Feed;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::sync::{Mutex, OnceLock};

    /// What the reader thread is told, behind a lock: pipes to take on,
    /// and whether it is running.
    struct Shared {
        new: Vec<(OwnedFd, Feed)>,
        running: bool,
        /// the write end of the pipe that wakes the reader's `poll`
        wake_w: i32,
        wake_r: i32,
    }

    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();

    fn shared() -> Option<&'static Mutex<Shared>> {
        if let Some(s) = SHARED.get() {
            return Some(s);
        }
        let mut fds = [0i32; 2];
        // SAFETY: pipe(2) writes two descriptors into the array.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return None;
        }
        for fd in fds {
            nonblocking(fd);
            // SAFETY: fcntl on a descriptor just made.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        let made = Mutex::new(Shared {
            new: Vec::new(),
            running: false,
            wake_r: fds[0],
            wake_w: fds[1],
        });
        if SHARED.set(made).is_err() {
            // another thread made it first: these two are not needed
            // SAFETY: both are ours and closed once.
            unsafe {
                libc::close(fds[0]);
                libc::close(fds[1]);
            }
        }
        SHARED.get()
    }

    fn nonblocking(fd: i32) {
        // SAFETY: fcntl on a descriptor the caller owns.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
    }

    pub(super) fn watch(fd: OwnedFd, feed: Feed) {
        let Some(sh) = shared() else {
            // no wake pipe: a thread for this one, as before
            let file = std::fs::File::from(fd);
            std::thread::spawn(move || super::drain(file, feed.sink));
            return;
        };
        nonblocking(fd.as_raw_fd());
        let mut s = sh.lock().unwrap_or_else(|e| e.into_inner());
        s.new.push((fd, feed));
        if s.running {
            // SAFETY: a byte into our own non-blocking pipe; a full pipe
            // already wakes the reader.
            unsafe { libc::write(s.wake_w, [1u8].as_ptr().cast(), 1) };
            return;
        }
        match std::thread::Builder::new()
            .name("olang-proc-reader".into())
            .spawn(move || run(sh))
        {
            Ok(_) => s.running = true,
            Err(_) => {
                // no thread to be had: read these on threads of their own
                let pending = std::mem::take(&mut s.new);
                drop(s);
                for (fd, feed) in pending {
                    let file = std::fs::File::from(fd);
                    std::thread::spawn(move || super::drain(file, feed.sink));
                }
            }
        }
    }

    /// The reader: every watched pipe, until none is left.
    fn run(sh: &'static Mutex<Shared>) {
        let mut pipes: Vec<(OwnedFd, Feed)> = Vec::new();
        let mut chunk = vec![0u8; 65536];
        let wake_r = sh.lock().unwrap_or_else(|e| e.into_inner()).wake_r;
        loop {
            {
                let mut s = sh.lock().unwrap_or_else(|e| e.into_inner());
                pipes.append(&mut s.new);
                if pipes.is_empty() {
                    s.running = false;
                    return;
                }
            }
            let mut fds: Vec<libc::pollfd> = Vec::with_capacity(pipes.len() + 1);
            fds.push(libc::pollfd {
                fd: wake_r,
                events: libc::POLLIN,
                revents: 0,
            });
            for (fd, _) in &pipes {
                fds.push(libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
            // SAFETY: `fds` is a live array of `fds.len()` pollfds.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if n < 0 {
                continue; // EINTR
            }
            if fds[0].revents != 0 {
                let mut sink = [0u8; 64];
                // SAFETY: reads into a local buffer from our own pipe.
                while unsafe { libc::read(wake_r, sink.as_mut_ptr().cast(), sink.len()) } > 0 {}
            }
            // the pipes that ended, highest first so removing keeps indexes
            let mut ended: Vec<(usize, bool)> = Vec::new();
            for (i, pfd) in fds.iter().enumerate().skip(1) {
                if pfd.revents == 0 {
                    continue;
                }
                let at = i - 1;
                let (fd, feed) = &mut pipes[at];
                // read what is there now, until it would block or for a
                // few chunks (a pipe that floods does not starve the rest)
                for _ in 0..16 {
                    // SAFETY: reads into `chunk`, which is `chunk.len()` long.
                    let r = unsafe {
                        libc::read(fd.as_raw_fd(), chunk.as_mut_ptr().cast(), chunk.len())
                    };
                    if r > 0 {
                        if !feed.push(&chunk[..r as usize]) {
                            ended.push((at, false));
                            break;
                        }
                        if (r as usize) < chunk.len() {
                            break; // drained for now: poll says when there is more
                        }
                        continue;
                    }
                    if r == 0 {
                        ended.push((at, true));
                        break;
                    }
                    let e = std::io::Error::last_os_error();
                    match e.kind() {
                        std::io::ErrorKind::Interrupted => {}
                        std::io::ErrorKind::WouldBlock => break,
                        _ => {
                            ended.push((at, false));
                            break;
                        }
                    }
                }
            }
            ended.sort_by_key(|e| std::cmp::Reverse(e.0));
            for (at, clean) in ended {
                let (fd, feed) = pipes.swap_remove(at);
                drop(fd); // closed before the end is said
                feed.finish(clean);
            }
        }
    }

    /// How many pipes the reader holds now, and whether it runs (tests).
    #[cfg(test)]
    pub(super) fn running() -> bool {
        shared()
            .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).running)
            .unwrap_or(false)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;

    fn lines_of(script: &str) -> Vec<String> {
        let mut c = Command::new("sh")
            .args(["-c", script])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = mpsc::channel();
        watch(c.stdout.take().unwrap(), Sink::Lines(tx));
        let out: Vec<String> = rx.iter().map(|l| String::from_utf8(l).unwrap()).collect();
        c.wait().unwrap();
        out
    }

    #[test]
    fn lines_come_whole_and_in_order_and_a_last_one_without_a_newline_too() {
        let out = lines_of(
            "i=0; while [ $i -lt 5000 ]; do echo \"line $i\"; i=$((i+1)); done; printf 'end\\r\\nlast'",
        );
        assert_eq!(out.len(), 5002);
        for (i, l) in out.iter().take(5000).enumerate() {
            assert_eq!(l, &format!("line {i}"));
        }
        assert_eq!(out[5000], "end");
        assert_eq!(out[5001], "last");
    }

    #[test]
    fn many_children_are_read_by_one_thread_at_once() {
        let mut kids = Vec::new();
        for k in 0..12 {
            let mut c = Command::new("sh")
                .args([
                    "-c",
                    &format!("for i in 1 2 3; do echo {k}-$i; sleep 0.05; done"),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let (tx, rx) = mpsc::channel();
            watch(c.stdout.take().unwrap(), Sink::Lines(tx));
            let err = Arc::new(Mutex::new(Vec::new()));
            watch(
                c.stderr.take().unwrap(),
                Sink::Bounded {
                    into: err.clone(),
                    keep: 1024,
                },
            );
            kids.push((c, rx, k));
        }
        assert!(unix::running());
        for (mut c, rx, k) in kids {
            let got: Vec<String> = rx.iter().map(|l| String::from_utf8(l).unwrap()).collect();
            assert_eq!(
                got,
                vec![format!("{k}-1"), format!("{k}-2"), format!("{k}-3")]
            );
            c.wait().unwrap();
        }
    }

    #[test]
    fn frames_are_cut_by_their_content_length() {
        let mut c = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, rx) = mpsc::channel();
        watch(c.stdout.take().unwrap(), Sink::Frames(tx));
        let mut input = c.stdin.take().unwrap();
        let bodies = ["{\"a\":1}", "", "héllo\nworld"];
        for b in bodies {
            write!(
                input,
                "Content-Length: {}\r\nContent-Type: x\r\n\r\n{}",
                b.len(),
                b
            )
            .unwrap();
        }
        drop(input);
        let got: Vec<String> = rx.iter().map(|l| String::from_utf8(l).unwrap()).collect();
        assert_eq!(got, bodies);
        c.wait().unwrap();
    }

    #[test]
    fn a_flood_of_stderr_keeps_its_last_part() {
        let mut c = Command::new("sh")
            .args([
                "-c",
                "i=0; while [ $i -lt 20000 ]; do echo \"line $i\" >&2; i=$((i+1)); done",
            ])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let err = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        watch(c.stderr.take().unwrap(), Sink::Whole { tx, tag: true });
        c.wait().unwrap();
        let (tag, all) = rx.recv().unwrap();
        assert!(tag);
        let bounded =
            crate::stdlib::proc::drain_bounded(&mut std::io::Cursor::new(all.clone()), 4096);
        let mut feed = Feed::new(Sink::Bounded {
            into: err.clone(),
            keep: 4096,
        });
        for part in all.chunks(65536) {
            feed.push(part);
        }
        feed.finish(true);
        assert_eq!(*err.lock().unwrap(), bounded);
        assert!(String::from_utf8_lossy(&bounded).ends_with("line 19999\n"));
    }
}
