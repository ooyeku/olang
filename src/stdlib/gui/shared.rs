//! Shared pictures: a frame one process draws on the GPU and another
//! shows, with nothing copied between them (macOS: IOSurface). Studio's
//! canvas runs a Toad game in a child process; the game's frames reach
//! Studio's window this way, at the display's rate.
//!
//! The producer (the game) draws each frame into one of three surfaces,
//! the next after the latest, and says which in a small header surface
//! once the GPU has finished it. An image whose source is
//! `("shared", id)`, `id` the header's IOSurface ID, shows the latest:
//! each new one is adopted as a live picture (a new picture id, so the
//! window's damage sees it change), and the header is told it was seen,
//! which paces the producer to the window that shows it, one frame ahead.
//!
//! The surfaces are global, so the window's process can look them up by
//! ID. Apple deprecates that as insecure: any process of this machine
//! that guesses an ID could read a frame or write the header. What is
//! shared is a game's picture in a development tool, and a bad header
//! only shows a wrong frame; passing mach ports instead would close it.

use super::canvas::Picture;
use super::gpu::Gpu;
use crate::ast::Value;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
#[allow(deprecated)]
use objc2_io_surface::{
    IOSurfaceLockOptions, IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceHeight,
    kIOSurfaceIsGlobal, kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MAGIC: u32 = 0x6f6c_7368;
const SLOTS: usize = 3;
/// 'RGBA': four bytes a pixel, in the order the GPU's Rgba8Unorm keeps.
const RGBA: i32 = 0x5247_4241;
/// A frame not seen for this long is taken as unseen (the pane hidden):
/// the next is drawn anyway, now and then.
const UNSEEN_MS: u128 = 250;
/// How many of a shared picture's frames stay adopted at once.
const KEEP: usize = 4;

/// The header, at the start of its surface: written by the producer
/// (`seq` last, once the frame's slot is finished) and by the window that
/// shows it (`seen`).
#[repr(C)]
struct Header {
    magic: AtomicU32,
    w: AtomicU32,
    h: AtomicU32,
    slot: AtomicU32,
    ids: [AtomicU32; SLOTS],
    _pad: u32,
    seq: AtomicU64,
    seen: AtomicU64,
}

/// A surface held locked, so its memory stays at one address.
struct Mem {
    surface: CFRetained<IOSurfaceRef>,
    base: *const Header,
}

// the header is atomics only, read and written from any thread
unsafe impl Send for Mem {}
unsafe impl Sync for Mem {}

impl Mem {
    fn new(surface: CFRetained<IOSurfaceRef>) -> Option<Mem> {
        let r = unsafe { surface.lock(IOSurfaceLockOptions::AvoidSync, std::ptr::null_mut()) };
        if r != 0 {
            return None;
        }
        let base = surface.base_address().as_ptr() as *const Header;
        Some(Mem { surface, base })
    }

    fn header(&self) -> &Header {
        unsafe { &*self.base }
    }
}

impl Drop for Mem {
    fn drop(&mut self) {
        unsafe {
            self.surface
                .unlock(IOSurfaceLockOptions::AvoidSync, std::ptr::null_mut());
        }
    }
}

#[allow(deprecated)]
fn surface(w: u32, h: u32, bpe: i32, format: Option<i32>) -> Option<CFRetained<IOSurfaceRef>> {
    let (kw, kh, kb, kp, kg) = unsafe {
        (
            kIOSurfaceWidth,
            kIOSurfaceHeight,
            kIOSurfaceBytesPerElement,
            kIOSurfacePixelFormat,
            kIOSurfaceIsGlobal,
        )
    };
    let (nw, nh, nb) = (
        CFNumber::new_i32(w as i32),
        CFNumber::new_i32(h as i32),
        CFNumber::new_i32(bpe),
    );
    let np = CFNumber::new_i32(format.unwrap_or(0));
    let mut keys: Vec<&CFString> = vec![kw, kh, kb, kg];
    let mut vals: Vec<&CFType> = vec![
        nw.as_ref(),
        nh.as_ref(),
        nb.as_ref(),
        CFBoolean::new(true).as_ref(),
    ];
    if format.is_some() {
        keys.push(kp);
        vals.push(np.as_ref());
    }
    let props = CFDictionary::from_slices(&keys, &vals);
    unsafe { IOSurfaceRef::new(props.as_opaque()) }
}

/// A Metal texture on `surface`, as a wgpu texture of this device.
fn texture_on(
    gpu: &Gpu,
    surface: &IOSurfaceRef,
    w: u32,
    h: u32,
    label: &'static str,
) -> Option<wgpu::Texture> {
    use wgpu::hal::api::Metal;
    let raw: Retained<ProtocolObject<dyn MTLTexture>> = unsafe {
        let dev = gpu.device.as_hal::<Metal>()?;
        let d = MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::RGBA8Unorm,
            w as usize,
            h as usize,
            false,
        );
        d.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        dev.raw_device()
            .newTextureWithDescriptor_iosurface_plane(&d, surface, 0)?
    };
    let hal = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            raw,
            wgpu::TextureFormat::Rgba8Unorm,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent {
                width: w,
                height: h,
                depth: 1,
            },
            None,
        )
    };
    let desc = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    Some(unsafe {
        gpu.device
            .create_texture_from_hal::<Metal>(hal, &desc, wgpu::TextureUses::UNINITIALIZED)
    })
}

// ── the producer: a game's frames ─────────────────────────────────────

struct Producer {
    head: Arc<Mem>,
    size: (u32, u32),
    slots: Vec<(CFRetained<IOSurfaceRef>, wgpu::Texture)>,
    /// the frames given to the GPU, and the last the header says
    sent: u64,
    next: usize,
    done: Arc<AtomicU64>,
    at: std::time::Instant,
}

unsafe impl Send for Producer {}

static PRODUCERS: Mutex<Option<HashMap<String, Producer>>> = Mutex::new(None);

fn producers<T>(f: impl FnOnce(&mut HashMap<String, Producer>) -> T) -> T {
    let mut g = PRODUCERS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

/// Copy `src` (Rgba8Unorm, `w` × `h`) into the next of `name`'s shared
/// surfaces, in `enc`. The header says it once [`submitted`]'s work is
/// done. False when it cannot be shared (no surfaces, or not Metal).
pub fn copy_in(
    name: &str,
    gpu: &Gpu,
    enc: &mut wgpu::CommandEncoder,
    src: &wgpu::Texture,
    w: u32,
    h: u32,
) -> bool {
    producers(|ps| {
        if !ps.contains_key(name) {
            let Some(head) = surface(256, 1, 1, None).and_then(Mem::new) else {
                return false;
            };
            head.header().magic.store(MAGIC, Ordering::Release);
            ps.insert(
                name.to_string(),
                Producer {
                    head: Arc::new(head),
                    size: (0, 0),
                    slots: Vec::new(),
                    sent: 0,
                    next: 0,
                    done: Arc::new(AtomicU64::new(0)),
                    at: std::time::Instant::now(),
                },
            );
        }
        let p = ps.get_mut(name).expect("made");
        if p.size != (w, h) {
            let mut slots = Vec::new();
            for _ in 0..SLOTS {
                let Some(s) = surface(w, h, 4, Some(RGBA)) else {
                    return false;
                };
                let Some(t) = texture_on(gpu, &s, w, h, "shared picture") else {
                    return false;
                };
                slots.push((s, t));
            }
            p.slots = slots;
            p.size = (w, h);
        }
        let hd = p.head.header();
        let next = (hd.slot.load(Ordering::Acquire) as usize + 1) % SLOTS;
        enc.copy_texture_to_texture(
            src.as_image_copy(),
            p.slots[next].1.as_image_copy(),
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        p.sent += 1;
        p.next = next;
        true
    })
}

/// After the submit that holds [`copy_in`]'s copy: once the GPU has done
/// it, the header names the new frame.
pub fn submitted(name: &str, gpu: &Gpu) {
    producers(|ps| {
        let Some(p) = ps.get_mut(name) else { return };
        let head = p.head.clone();
        let slot = p.next;
        let ids: Vec<u32> = p.slots.iter().map(|(s, _)| s.id()).collect();
        let (w, h, seq, done) = (p.size.0, p.size.1, p.sent, p.done.clone());
        p.at = std::time::Instant::now();
        gpu.queue.on_submitted_work_done(move || {
            let hd = head.header();
            hd.w.store(w, Ordering::Release);
            hd.h.store(h, Ordering::Release);
            for (i, id) in ids.iter().enumerate() {
                hd.ids[i].store(*id, Ordering::Release);
            }
            hd.slot.store(slot as u32, Ordering::Release);
            hd.seq.store(seq, Ordering::Release);
            done.store(seq, Ordering::Release);
        });
    });
}

/// Whether `name`, when it is shared, wants a new frame: the last one
/// done and seen (or unseen a while). None when it is not shared.
pub fn wanted(name: &str) -> Option<bool> {
    if let Some(gpu) = super::gpu::current() {
        let _ = gpu.device.poll(wgpu::PollType::Poll);
    }
    producers(|ps| {
        let p = ps.get(name)?;
        let hd = p.head.header();
        let done = p.done.load(Ordering::Acquire);
        if done < p.sent {
            // the last still on the GPU
            return Some(false);
        }
        // one frame ahead of the window: the next is made while it shows
        // the last (three surfaces: the one shown, the latest, the next)
        Some(hd.seen.load(Ordering::Acquire) + 1 >= done || p.at.elapsed().as_millis() >= UNSEEN_MS)
    })
}

/// The ID of `name`'s header surface, once it has drawn a frame: what an
/// image's `("shared", id)` names.
/// `()` for `name`: the only one, when just one is shared.
pub fn id_of(name: Option<&str>) -> Option<u32> {
    producers(|ps| match name {
        Some(n) => ps.get(n).map(|p| p.head.surface.id()),
        None if ps.len() == 1 => ps.values().next().map(|p| p.head.surface.id()),
        None => None,
    })
}

// ── the window: a shared picture shown ─────────────────────────────────

struct Shown {
    head: Mem,
    views: HashMap<u32, wgpu::TextureView>,
    seq: u64,
    pic: Option<Arc<Picture>>,
    ids: Vec<u64>,
}

static SHOWN: Mutex<Option<HashMap<u32, Shown>>> = Mutex::new(None);

/// The header ID in a shared source, `("shared", id)`.
pub fn id_in(src: &Value) -> Option<u32> {
    match src {
        Value::Tuple(t) if t.len() == 2 => match (&t[0], &t[1]) {
            (Value::String(k), Value::Integer(n)) if k.as_str() == "shared" && *n > 0 => {
                Some(*n as u32)
            }
            _ => None,
        },
        _ => None,
    }
}

/// The latest frame of the shared picture `id`, adopted as a live
/// picture, and said seen. None before its first frame, or when the
/// producer is gone.
pub fn current(id: u32) -> Option<Arc<Picture>> {
    let gpu = super::gpu::current()?;
    let mut g = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    let all = g.get_or_insert_with(HashMap::new);
    if !all.contains_key(&id) {
        let head = Mem::new(IOSurfaceRef::lookup(id)?)?;
        if head.header().magic.load(Ordering::Acquire) != MAGIC {
            return None;
        }
        all.insert(
            id,
            Shown {
                head,
                views: HashMap::new(),
                seq: 0,
                pic: None,
                ids: Vec::new(),
            },
        );
    }
    let s = all.get_mut(&id).expect("made");
    let hd = s.head.header();
    let seq = hd.seq.load(Ordering::Acquire);
    if seq != s.seq && seq > 0 {
        let (w, h) = (hd.w.load(Ordering::Acquire), hd.h.load(Ordering::Acquire));
        let slot = hd.slot.load(Ordering::Acquire) as usize % SLOTS;
        let sid = hd.ids[slot].load(Ordering::Acquire);
        // surfaces of an older size are let go
        let live: Vec<u32> = hd.ids.iter().map(|i| i.load(Ordering::Acquire)).collect();
        s.views.retain(|k, _| live.contains(k));
        if !s.views.contains_key(&sid) {
            let surf = IOSurfaceRef::lookup(sid)?;
            let t = texture_on(&gpu, &surf, w, h, "shared picture shown")?;
            s.views
                .insert(sid, t.create_view(&wgpu::TextureViewDescriptor::default()));
        }
        let pid = super::canvas::next_picture_id();
        gpu.adopt_picture(pid, s.views[&sid].clone());
        s.ids.push(pid);
        let n = s.ids.len().saturating_sub(KEEP);
        for old in s.ids.drain(..n) {
            gpu.forget_picture(old);
        }
        s.pic = Some(Arc::new(Picture {
            id: pid,
            width: w,
            height: h,
            rgba: Vec::new(),
        }));
        s.seq = seq;
    }
    hd.seen.store(s.seq, Ordering::Release);
    s.pic.clone()
}
