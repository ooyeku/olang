//! A window captured as it draws (`gui.capture`): each frame the window
//! presents is drawn again, smaller, into a texture of the capture's own
//! and copied to a buffer without waiting for the GPU (the surface's part:
//! [`super::gpu::Surface::capture_frame`]); this module is the other
//! half, a thread of the capture's own that takes each buffer once the
//! GPU has filled it, writes it as a PNG and says when it showed.
//!
//! The render thread never waits: it encodes one small pass and one copy
//! a frame, hands the buffer over a channel, and draws on. A window that
//! draws nothing is captured not at all. When the writer falls behind
//! (more than [`MAX_IN_FLIGHT`] frames not yet written) a frame is left
//! out and counted, never queued without bound.
//!
//! What is written, under `dir`:
//!
//!   frames/NNNNNN.png   a frame, RGB, 8 bits a channel
//!   index.jsonl         a line a frame, appended: `{"t":ms,"f":n}`
//!
//! `t` counts from the capture's start. (OL-587; OlangStudio's Recorder.)

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Frames handed to the writer and not yet written, past which a frame
/// is left out.
pub const MAX_IN_FLIGHT: usize = 8;

/// What a capture is asked for.
#[derive(Clone, Debug)]
pub struct Opts {
    /// The folder the frames and the index go in.
    pub dir: PathBuf,
    /// At most this many frames a second.
    pub fps: f32,
    /// The frame's size against the window's pixels (0.5: a Retina
    /// window's points).
    pub scale: f32,
    /// One movie (`movie.mov`, H.264 by the system's hardware encoder)
    /// in place of a PNG a frame. macOS.
    pub movie: bool,
}

/// What a capture did, so far or in all.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub frames: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub ms: u64,
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
}

/// The counts both threads keep.
#[derive(Default)]
pub struct Stats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub dropped: AtomicU64,
    pub width: AtomicU64,
    pub height: AtomicU64,
    pub in_flight: AtomicUsize,
    pub failed: std::sync::Mutex<Option<String>>,
}

/// A frame on its way from the GPU: its buffer (mapped when `ready`), its
/// size, its rows' stride, when it showed.
pub struct Pending {
    pub t_ms: u64,
    pub buf: wgpu::Buffer,
    pub w: u32,
    pub h: u32,
    pub row: u32,
    pub ready: Arc<AtomicBool>,
    pub failed: Arc<AtomicBool>,
}

pub enum Msg {
    Frame(Pending),
    Stop,
}

/// The writer's end of a capture.
pub struct Writer {
    pub tx: crossbeam_channel::Sender<Msg>,
    /// Buffers the writer is done with, to use again.
    pub back: crossbeam_channel::Receiver<wgpu::Buffer>,
    pub stats: Arc<Stats>,
    pub started: Instant,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    /// The writer's thread begun for `opts.dir` (made, with its `frames`).
    pub fn start(device: wgpu::Device, opts: &Opts) -> Result<Writer, String> {
        let frames = opts.dir.join("frames");
        std::fs::create_dir_all(&frames)
            .map_err(|e| format!("gui.capture: {}: {e}", frames.display()))?;
        let index = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(opts.dir.join("index.jsonl"))
            .map_err(|e| format!("gui.capture: the index: {e}"))?;
        let (tx, rx) = crossbeam_channel::unbounded::<Msg>();
        let (back_tx, back) = crossbeam_channel::unbounded::<wgpu::Buffer>();
        let stats = Arc::new(Stats::default());
        let st = stats.clone();
        let thread = std::thread::Builder::new()
            .name("gui-capture".to_string())
            .spawn({
                let o = opts.clone();
                move || write_loop(device, rx, back_tx, frames, index, st, o)
            })
            .map_err(|e| format!("gui.capture: its thread: {e}"))?;
        Ok(Writer {
            tx,
            back,
            stats,
            started: Instant::now(),
            thread: Some(thread),
        })
    }

    /// What it did so far.
    pub fn summary(&self) -> Summary {
        Summary {
            frames: self.stats.frames.load(Ordering::Relaxed),
            bytes: self.stats.bytes.load(Ordering::Relaxed),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
            ms: self.started.elapsed().as_millis() as u64,
            width: self.stats.width.load(Ordering::Relaxed) as u32,
            height: self.stats.height.load(Ordering::Relaxed) as u32,
            error: self
                .stats
                .failed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    /// The frames on their way written, the thread ended: what it did.
    pub fn stop(mut self) -> Summary {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.summary()
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // a window closed while it was captured: the thread ends by itself
        let _ = self.tx.send(Msg::Stop);
    }
}

fn write_loop(
    device: wgpu::Device,
    rx: crossbeam_channel::Receiver<Msg>,
    back: crossbeam_channel::Sender<wgpu::Buffer>,
    frames: PathBuf,
    mut index: std::fs::File,
    stats: Arc<Stats>,
    opts: Opts,
) {
    // each with when it came: one whose copy never completes is let go a second after
    let mut pending: VecDeque<(Pending, Instant)> = VecDeque::new();
    let mut stopping = false;
    let mut n: u64 = 0;
    // the movie, begun at the first frame (its size is the frame's)
    #[cfg(target_os = "macos")]
    let mut movie: Option<super::avf::MovieWriter> = None;
    let mut last_t: u64 = 0;
    let fail = |stats: &Stats, e: String| {
        *stats.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
    };
    loop {
        // what the render thread handed over since the last look
        let first = if pending.is_empty() && !stopping {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(m) => Some(m),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                // the capture was dropped with its window
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => Some(Msg::Stop),
            }
        } else {
            rx.try_recv().ok()
        };
        let mut next = first;
        while let Some(m) = next {
            match m {
                Msg::Frame(p) => pending.push_back((p, Instant::now())),
                Msg::Stop => stopping = true,
            }
            next = rx.try_recv().ok();
        }
        if pending.is_empty() {
            if stopping {
                // the movie's file finished: its bytes are what was written
                #[cfg(target_os = "macos")]
                if let Some(m) = movie.take() {
                    match m.finish(last_t as i64 + 17) {
                        Ok(bytes) => stats.bytes.store(bytes, Ordering::Relaxed),
                        Err(e) => fail(&stats, e),
                    }
                }
                return;
            }
            continue;
        }
        // the GPU's finished copies say so now (their map's callback runs here)
        let _ = device.poll(wgpu::PollType::Poll);
        let (front_ready, front_old) = pending.front().map_or((false, false), |(p, came)| {
            (
                p.ready.load(Ordering::Acquire) || p.failed.load(Ordering::Acquire),
                came.elapsed() > Duration::from_secs(1),
            )
        });
        if !front_ready {
            // a frame's copy takes a millisecond or two; one that never
            // comes (the device lost) is let go a second after it came
            if front_old {
                if let Some((p, _)) = pending.pop_front() {
                    stats.in_flight.fetch_sub(1, Ordering::Relaxed);
                    stats.dropped.fetch_add(1, Ordering::Relaxed);
                    drop(p);
                }
            } else {
                std::thread::sleep(Duration::from_micros(500));
            }
            continue;
        }
        let Some((p, _)) = pending.pop_front() else {
            continue;
        };
        stats.in_flight.fetch_sub(1, Ordering::Relaxed);
        if p.failed.load(Ordering::Acquire) {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        // a movie: the frame's rows handed to the hardware encoder
        #[cfg(target_os = "macos")]
        if opts.movie {
            let slice = p.buf.slice(..);
            let took = match slice.get_mapped_range() {
                Err(_) => Ok(false),
                Ok(data) => {
                    if movie.is_none() {
                        let path = opts.dir.join("movie.mov");
                        match super::avf::MovieWriter::start(&path.to_string_lossy(), p.w, p.h, opts.fps) {
                            Ok(m) => movie = Some(m),
                            Err(e) => fail(&stats, e),
                        }
                    }
                    match movie.as_mut() {
                        // (a frame of another size, the window resized past what the capture held: left out)
                        Some(m) if (m.width, m.height) == (p.w, p.h) => m.append(p.t_ms as i64, &data, p.row as usize),
                        Some(_) => Ok(false),
                        None => Err("the movie could not be begun".to_string()),
                    }
                }
            };
            p.buf.unmap();
            let _ = back.send(p.buf);
            match took {
                Ok(true) => {
                    n += 1;
                    last_t = p.t_ms;
                    let _ = writeln!(index, "{{\"t\":{},\"f\":{}}}", p.t_ms, n);
                    stats.frames.store(n, Ordering::Relaxed);
                    stats.width.store(p.w as u64, Ordering::Relaxed);
                    stats.height.store(p.h as u64, Ordering::Relaxed);
                    // how much is written so far, now and then (a limit watches it)
                    if n % 30 == 0
                        && let Ok(md) = std::fs::metadata(opts.dir.join("movie.mov"))
                    {
                        stats.bytes.store(md.len(), Ordering::Relaxed);
                    }
                }
                Ok(false) => {
                    stats.dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    fail(&stats, e);
                    while let Some((q, _)) = pending.pop_front() {
                        stats.in_flight.fetch_sub(1, Ordering::Relaxed);
                        q.buf.unmap();
                    }
                    if let Some(m) = movie.take() {
                        let _ = m.finish(last_t as i64 + 17);
                    }
                    return;
                }
            }
            continue;
        }
        let png = {
            let slice = p.buf.slice(..);
            match slice.get_mapped_range() {
                Ok(data) => Some(png_rgb(p.w, p.h, p.row as usize, &data)),
                Err(_) => None,
            }
        };
        p.buf.unmap();
        let _ = back.send(p.buf);
        let Some(png) = png else {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        n += 1;
        let path = frames.join(format!("{n:06}.png"));
        let wrote = std::fs::write(&path, &png)
            .and_then(|_| writeln!(index, "{{\"t\":{},\"f\":{}}}", p.t_ms, n));
        match wrote {
            Ok(()) => {
                stats.frames.store(n, Ordering::Relaxed);
                stats.bytes.fetch_add(png.len() as u64, Ordering::Relaxed);
                stats.width.store(p.w as u64, Ordering::Relaxed);
                stats.height.store(p.h as u64, Ordering::Relaxed);
            }
            Err(e) => {
                // a full disk: said once, and nothing more is written
                *stats.failed.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(format!("a frame could not be written: {e}"));
                while let Some((q, _)) = pending.pop_front() {
                    stats.in_flight.fetch_sub(1, Ordering::Relaxed);
                    q.buf.unmap();
                }
                return;
            }
        }
    }
}

/// `w × h` pixels (RGBA rows `row` bytes apart) as a PNG: RGB, each row
/// against the row over it (the Up filter: a window is mostly flat
/// colour, so most bytes are zero), deflated at the fastest level. A
/// 1440 × 900 frame takes a few milliseconds.
pub fn png_rgb(w: u32, h: u32, row: usize, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let line = w * 3;
    let mut raw = Vec::with_capacity((line + 1) * h);
    let mut prev = vec![0u8; line];
    let mut cur = vec![0u8; line];
    for y in 0..h {
        let src = &rgba[y * row..y * row + w * 4];
        for (d, s) in cur.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
            d[0] = s[0];
            d[1] = s[1];
            d[2] = s[2];
        }
        raw.push(2u8);
        raw.extend(cur.iter().zip(prev.iter()).map(|(c, p)| c.wrapping_sub(*p)));
        std::mem::swap(&mut prev, &mut cur);
    }
    let mut z = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(raw.len() / 8),
        flate2::Compression::fast(),
    );
    let _ = z.write_all(&raw);
    let data = z.finish().unwrap_or_default();
    let mut out = Vec::with_capacity(data.len() + 64);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &data);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

/// The capture's size for a kept frame of `w × h` pixels at `scale`.
pub fn scaled(w: u32, h: u32, scale: f32) -> (u32, u32) {
    let s = scale.clamp(0.1, 1.0);
    (
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
    )
}

/// [`scaled`] for a movie: even numbers (H.264 takes no odd side), at
/// least 16.
pub fn scaled_even(w: u32, h: u32, scale: f32) -> (u32, u32) {
    let (cw, ch) = scaled(w, h, scale);
    ((cw / 2 * 2).max(16), (ch / 2 * 2).max(16))
}

/// The least time between two frames at `fps`.
pub fn every(fps: f32) -> Duration {
    Duration::from_secs_f32(1.0 / fps.clamp(1.0, 120.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_is_read_back_as_its_pixels() {
        // 3 × 2, rows 16 bytes apart (padding after each): the padding is not in it
        let (w, h, row) = (3u32, 2u32, 16usize);
        let mut px = vec![0u8; row * 2];
        for y in 0..2usize {
            for x in 0..3usize {
                let o = y * row + x * 4;
                px[o] = (10 * (x + 1)) as u8;
                px[o + 1] = (100 + y * 50) as u8;
                px[o + 2] = (x * y * 40) as u8;
                px[o + 3] = 255;
            }
        }
        let png = png_rgb(w, h, row, &px);
        let pm = tiny_skia::Pixmap::decode_png(&png).expect("a PNG tiny-skia reads");
        assert_eq!((pm.width(), pm.height()), (3, 2));
        let got = pm.pixel(2, 1).expect("a pixel");
        assert_eq!(
            (got.red(), got.green(), got.blue(), got.alpha()),
            (30, 150, 80, 255)
        );
        let got = pm.pixel(0, 0).expect("a pixel");
        assert_eq!((got.red(), got.green(), got.blue()), (10, 100, 0));
    }

    #[test]
    fn sizes_and_rates() {
        assert_eq!(scaled(2880, 1728, 0.5), (1440, 864));
        assert_eq!(scaled(3, 3, 0.01), (1, 1));
        assert_eq!(scaled(100, 50, 4.0), (100, 50));
        // a movie's sides are even, and never tiny
        assert_eq!(scaled_even(2881, 1729, 1.0), (2880, 1728));
        assert_eq!(scaled_even(1441, 865, 0.5), (720, 432));
        assert_eq!(scaled_even(10, 10, 0.1), (16, 16));
        assert_eq!(every(60.0), Duration::from_secs_f32(1.0 / 60.0));
        assert_eq!(every(0.0), Duration::from_secs(1));
    }
}
