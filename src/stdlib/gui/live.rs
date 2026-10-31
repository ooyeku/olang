//! Live pictures: textures another part of the engine draws on the GPU
//! (Toad's frames), shown by an image node whose source is
//! `("live", name)`. Each publish is a new picture id, so the window's
//! damage sees a changed picture; the image node asks for the next
//! refresh while it is shown, so a live picture moves at the display's
//! rate. A window drawn in software (headless, or without a GPU) has no
//! pixels for it and draws nothing there.

use super::canvas::Picture;
use crate::ast::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A name's pictures: the latest, the ids still adopted (a window may be
/// drawing one of the last few while the next is published), whether a
/// window has drawn the latest, and when it was published.
struct Entry {
    pic: Arc<Picture>,
    ids: Vec<u64>,
    shown: bool,
    at: std::time::Instant,
}

static LIVE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

/// A frame no window drew for this long is taken as unseen (the window
/// hidden, minimised, or never made): the next is wanted anyway, now and
/// then, so a window that comes back finds a recent one.
const UNSEEN_MS: u128 = 250;

/// How many of a name's pictures stay on the GPU.
const KEEP: usize = 4;

/// The name in a live source, `("live", name)`.
pub fn name_of(src: &Value) -> Option<String> {
    match src {
        Value::Tuple(t) if t.len() == 2 => match (&t[0], &t[1]) {
            (Value::String(k), Value::String(n)) if k.as_str() == "live" => Some(n.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The picture last published as `name`.
pub fn current(name: &str) -> Option<Arc<Picture>> {
    LIVE.lock().ok()?.as_ref()?.get(name).map(|e| e.pic.clone())
}

/// Publish `view` (an RGBA8 texture of `w` × `h`, display-encoded and
/// opaque, made with TEXTURE_BINDING) as `name`'s picture: answers its id.
pub fn publish(name: &str, view: wgpu::TextureView, w: u32, h: u32) -> u64 {
    let id = super::canvas::next_picture_id();
    let pic = Arc::new(Picture {
        id,
        width: w,
        height: h,
        rgba: Vec::new(),
    });
    let Some(gpu) = super::gpu::current() else {
        return id;
    };
    gpu.adopt_picture(id, view);
    let stale: Vec<u64> = match LIVE.lock() {
        Ok(mut m) => {
            let e = m.get_or_insert_with(HashMap::new).entry(name.to_string()).or_insert_with(|| Entry {
                pic: pic.clone(),
                ids: Vec::new(),
                shown: false,
                at: std::time::Instant::now(),
            });
            e.pic = pic;
            e.shown = false;
            e.at = std::time::Instant::now();
            e.ids.push(id);
            let n = e.ids.len().saturating_sub(KEEP);
            e.ids.drain(..n).collect()
        }
        Err(_) => Vec::new(),
    };
    for old in stale {
        gpu.forget_picture(old);
    }
    id
}

/// Publish a frame's pixels (RGBA8, `w` × `h`, display-encoded and
/// opaque) as `name`'s picture: for a window drawn in software (headless,
/// Studio's canvas), which draws pixels, not textures. Answers its id.
pub fn publish_pixels(name: &str, w: u32, h: u32, rgba: Vec<u8>) -> u64 {
    let id = super::canvas::next_picture_id();
    let pic = Arc::new(Picture { id, width: w, height: h, rgba });
    if let Ok(mut m) = LIVE.lock() {
        let e = m.get_or_insert_with(HashMap::new).entry(name.to_string()).or_insert_with(|| Entry {
            pic: pic.clone(),
            ids: Vec::new(),
            shown: false,
            at: std::time::Instant::now(),
        });
        e.pic = pic;
        e.shown = false;
        e.at = std::time::Instant::now();
    }
    id
}

/// A window drew `name`'s picture `id`.
pub fn shown(name: &str, id: u64) {
    if let Ok(mut m) = LIVE.lock()
        && let Some(e) = m.as_mut().and_then(|m| m.get_mut(name))
        && e.pic.id == id
    {
        e.shown = true;
    }
}

/// Whether `name` wants a new frame: none yet, its latest drawn by a
/// window, or its latest unseen for a while. A game draws only then, so
/// it draws at the display's rate and not at all while it cannot be seen.
pub fn wanted(name: &str) -> bool {
    // a frame shared with another process: paced by the window there
    #[cfg(target_os = "macos")]
    if let Some(w) = super::shared::wanted(name) {
        return w;
    }
    match LIVE.lock().ok().as_ref().and_then(|m| m.as_ref()?.get(name).map(|e| (e.shown, e.at))) {
        None => true,
        Some((shown, at)) => shown || at.elapsed().as_millis() >= UNSEEN_MS,
    }
}


/// The shared picture in an image source, `("shared", id)`: its latest
/// frame, said seen (macOS; nothing elsewhere). None for any other source.
pub fn shared_in(src: &Value) -> Option<Option<Arc<Picture>>> {
    #[cfg(target_os = "macos")]
    {
        super::shared::id_in(src).map(super::shared::current)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = src;
        None
    }
}
