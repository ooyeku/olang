//! A movie played in a window (`gui.movie`, macOS): the system's player
//! (AVPlayer: its own clock, its hardware decoder, any rate, exact
//! seeks) decodes it, and each frame it has ready is put on the GPU and
//! published as a live picture ([`super::live`]), which an `image` node
//! whose source is `("live", name)` shows at the display's rate.
//!
//! A movie is a thread of its own: it owns the player, hears what is
//! asked of it over a channel, and says where it is in a shared record.
//! Nothing here runs on a window's thread. (OL-587; OlangStudio's
//! Recorder plays its recordings this way.)

use super::avf::{
    BGRA, CMTime, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferRelease,
    CVPixelBufferUnlockBaseAddress, class, file_url, kCVPixelBufferPixelFormatTypeKey, nsdict,
    nsnumber,
};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where a movie is, as its thread last said.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub ms: i64,
    pub length_ms: i64,
    pub playing: bool,
    pub ended: bool,
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    pub error: Option<String>,
}

enum Cmd {
    Play(f32),
    Pause,
    Seek(i64),
    Close,
}

struct Handle {
    tx: std::sync::mpsc::Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    live: String,
}

static MOVIES: Mutex<Option<HashMap<u64, Handle>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn with<T>(f: impl FnOnce(&mut HashMap<u64, Handle>) -> T) -> T {
    let mut g = MOVIES.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

/// The movie at `path` opened, paused at its start, its frames published
/// as the live picture `name`: its id.
pub fn open(path: &str, name: &str) -> Result<u64, String> {
    if !std::path::Path::new(path).is_file() {
        return Err(format!("gui.movie: no such movie: {path}"));
    }
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let status = Arc::new(Mutex::new(Status::default()));
    let (st, p, n) = (status.clone(), path.to_string(), name.to_string());
    std::thread::Builder::new()
        .name("gui-movie".into())
        .spawn(move || {
            if let Err(e) = play_loop(&p, &n, rx, &st) {
                st.lock().unwrap_or_else(|e| e.into_inner()).error = Some(e);
            }
        })
        .map_err(|e| format!("gui.movie: its thread: {e}"))?;
    with(|m| {
        m.insert(
            id,
            Handle {
                tx,
                status,
                live: name.to_string(),
            },
        )
    });
    Ok(id)
}

/// What is asked of movie `id`, then where it is.
pub fn ask(
    id: u64,
    play: Option<bool>,
    rate: Option<f32>,
    at: Option<i64>,
    close: bool,
) -> Result<(Status, String), String> {
    with(|m| {
        let Some(h) = m.get(&id) else {
            return Err(format!("gui.movie: no movie {id}"));
        };
        if let Some(t) = at {
            let _ = h.tx.send(Cmd::Seek(t.max(0)));
        }
        match play {
            Some(true) => {
                let _ = h.tx.send(Cmd::Play(rate.unwrap_or(1.0).clamp(0.1, 16.0)));
            }
            Some(false) => {
                let _ = h.tx.send(Cmd::Pause);
            }
            None => {
                if let Some(r) = rate {
                    // a rate alone changes how fast a playing movie goes
                    let playing = h.status.lock().unwrap_or_else(|e| e.into_inner()).playing;
                    if playing {
                        let _ = h.tx.send(Cmd::Play(r.clamp(0.1, 16.0)));
                    }
                }
            }
        }
        let mut s = h.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // (what was just asked is so at once, for the caller's next look)
        if let Some(t) = at {
            s.ms = t.max(0);
            s.ended = false;
        }
        if let Some(p) = play {
            s.playing = p;
        }
        let live = h.live.clone();
        if close {
            let _ = h.tx.send(Cmd::Close);
            m.remove(&id);
        }
        Ok((s, live))
    })
}

fn play_loop(
    path: &str,
    name: &str,
    rx: std::sync::mpsc::Receiver<Cmd>,
    status: &Arc<Mutex<Status>>,
) -> Result<(), String> {
    let url = file_url(path)?;
    // SAFETY: each message is the documented one for its class with live
    // objects of the documented kinds; every object is used on this
    // thread alone and released by its `Retained` when the loop ends.
    let (player, item, out) = unsafe {
        let item: Option<Retained<AnyObject>> =
            msg_send![class(c"AVPlayerItem")?, playerItemWithURL: &*url];
        let item = item.ok_or("the movie could not be opened")?;
        let attrs = nsdict(&[(kCVPixelBufferPixelFormatTypeKey, &*nsnumber(BGRA as i64)?)])?;
        let alloc: *mut AnyObject = msg_send![class(c"AVPlayerItemVideoOutput")?, alloc];
        // (+alloc then -init…: a raw pointer in, a retained object out)
        let made: *mut AnyObject = msg_send![alloc, initWithPixelBufferAttributes: &*attrs];
        let out = Retained::from_raw(made).ok_or("the movie's frames could not be asked for")?;
        let _: () = msg_send![&*item, addOutput: &*out];
        let player: Option<Retained<AnyObject>> =
            msg_send![class(c"AVPlayer")?, playerWithPlayerItem: &*item];
        let player = player.ok_or("the movie's player could not be made")?;
        // at its end it stops there (1: AVPlayerActionAtItemEndPause)
        let _: () = msg_send![&*player, setActionAtItemEnd: 1isize];
        (player, item, out)
    };
    let zero = CMTime::ms(0);
    let mut frames: u64 = 0;
    let mut idle = 0u32;
    loop {
        let mut closing = false;
        // what was asked since the last look
        loop {
            match rx.try_recv() {
                // SAFETY: as above.
                Ok(Cmd::Play(rate)) => unsafe {
                    // from the start again when it stands at its end
                    let t: CMTime = msg_send![&*player, currentTime];
                    let d: CMTime = msg_send![&*item, duration];
                    if d.to_ms() > 0 && t.to_ms() >= d.to_ms() - 20 {
                        let _: () = msg_send![&*player, seekToTime: zero, toleranceBefore: zero, toleranceAfter: zero];
                    }
                    let _: () = msg_send![&*player, setRate: rate];
                    idle = 0;
                },
                Ok(Cmd::Pause) => unsafe {
                    let _: () = msg_send![&*player, pause];
                    idle = 0;
                },
                Ok(Cmd::Seek(ms)) => unsafe {
                    let _: () = msg_send![&*player, seekToTime: CMTime::ms(ms), toleranceBefore: zero, toleranceAfter: zero];
                    idle = 0;
                },
                Ok(Cmd::Close) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    closing = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        if closing {
            // SAFETY: as above.
            unsafe {
                let _: () = msg_send![&*player, pause];
            }
            return Ok(());
        }
        let mut drew = false;
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: as above; the pixel buffer is ours to release, and
            // its bytes are read only while it is locked.
            unsafe {
                let t: CMTime = msg_send![&*player, currentTime];
                let has: Bool = msg_send![&*out, hasNewPixelBufferForItemTime: t];
                if has.as_bool() {
                    let pb: *mut c_void = msg_send![&*out, copyPixelBufferForItemTime: t, itemTimeForDisplay: std::ptr::null_mut::<CMTime>()];
                    if !pb.is_null() {
                        CVPixelBufferLockBaseAddress(pb, 1);
                        let (w, h) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb));
                        let stride = CVPixelBufferGetBytesPerRow(pb);
                        let base = CVPixelBufferGetBaseAddress(pb);
                        let px = if base.is_null() || w == 0 || h == 0 {
                            None
                        } else {
                            Some(std::slice::from_raw_parts(base, stride * h).to_vec())
                        };
                        CVPixelBufferUnlockBaseAddress(pb, 1);
                        CVPixelBufferRelease(pb);
                        if let Some(px) = px
                            && show(name, w as u32, h as u32, stride as u32, &px)
                        {
                            frames += 1;
                            drew = true;
                            let mut s = status.lock().unwrap_or_else(|e| e.into_inner());
                            s.width = w as u32;
                            s.height = h as u32;
                            s.frames = frames;
                        }
                    }
                }
                let rate: f32 = msg_send![&*player, rate];
                let d: CMTime = msg_send![&*item, duration];
                let err: *mut AnyObject = msg_send![&*item, error];
                let mut s = status.lock().unwrap_or_else(|e| e.into_inner());
                s.ms = t.to_ms();
                s.length_ms = d.to_ms();
                s.playing = rate != 0.0;
                s.ended = s.length_ms > 0 && s.ms >= s.length_ms - 20;
                if !err.is_null() && s.error.is_none() {
                    let dsc: Option<Retained<objc2_foundation::NSString>> =
                        msg_send![err, localizedDescription];
                    s.error = Some(dsc.map_or("the movie cannot be played".into(), |x| x.to_string()));
                }
            }
        });
        // a playing movie is looked at a few times a refresh; a still one
        // (paused, nothing new for a while) only now and then
        idle = if drew { 0 } else { idle.saturating_add(1) };
        let playing = status.lock().unwrap_or_else(|e| e.into_inner()).playing;
        std::thread::sleep(Duration::from_millis(if playing || idle < 200 { 4 } else { 30 }));
    }
}

/// A decoded frame (BGRA rows `stride` apart) put on the GPU and
/// published as `name`'s live picture. `false` with no GPU.
fn show(name: &str, w: u32, h: u32, stride: u32, px: &[u8]) -> bool {
    let Some(gpu) = super::gpu::current() else {
        return false;
    };
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gui movie"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        px,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    super::live::publish(name, view, w, h);
    true
}
