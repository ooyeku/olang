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

/// By name: the latest picture, and the ids still adopted (a window may
/// be drawing one of the last few while the next is published).
static LIVE: Mutex<Option<HashMap<String, (Arc<Picture>, Vec<u64>)>>> = Mutex::new(None);

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
    LIVE.lock().ok()?.as_ref()?.get(name).map(|e| e.0.clone())
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
            let e = m
                .get_or_insert_with(HashMap::new)
                .entry(name.to_string())
                .or_insert_with(|| (pic.clone(), Vec::new()));
            e.0 = pic;
            e.1.push(id);
            let n = e.1.len().saturating_sub(KEEP);
            e.1.drain(..n).collect()
        }
        Err(_) => Vec::new(),
    };
    for old in stale {
        gpu.forget_picture(old);
    }
    id
}
