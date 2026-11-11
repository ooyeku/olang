//! AVFoundation by hand (macOS): a movie written with the system's
//! hardware encoder as a window is captured ([`MovieWriter`]), and the
//! little of CoreMedia and CoreVideo that takes. No binding crate: the
//! classes are looked up by name and messaged, the constants are the
//! frameworks' own symbols.
//!
//! Every call here is made on the one thread that owns the object (a
//! capture's writer thread).

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use std::ffi::{CStr, c_void};

/// CoreMedia's time: `value / timescale` seconds.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

// SAFETY: CMTime's layout and its Objective-C encoding, `{?=qiIq}`.
unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[
            Encoding::LongLong,
            Encoding::Int,
            Encoding::UInt,
            Encoding::LongLong,
        ],
    );
}
// SAFETY: as above, by reference.
unsafe impl RefEncode for CMTime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

impl CMTime {
    /// `ms` milliseconds (a valid time: flag 1).
    pub fn ms(ms: i64) -> CMTime {
        CMTime {
            value: ms,
            timescale: 1000,
            flags: 1,
            epoch: 0,
        }
    }
    /// The time in milliseconds (0 when it is not a number).
    pub fn to_ms(self) -> i64 {
        if self.flags & 1 == 0 || self.timescale == 0 {
            0
        } else {
            (self.value as i128 * 1000 / self.timescale as i128) as i64
        }
    }
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: *const AnyObject;
    static AVFileTypeQuickTimeMovie: *const AnyObject;
    static AVVideoCodecKey: *const AnyObject;
    static AVVideoCodecTypeH264: *const AnyObject;
    static AVVideoWidthKey: *const AnyObject;
    static AVVideoHeightKey: *const AnyObject;
    static AVVideoCompressionPropertiesKey: *const AnyObject;
    static AVVideoAverageBitRateKey: *const AnyObject;
    static AVVideoMaxKeyFrameIntervalKey: *const AnyObject;
    static AVVideoExpectedSourceFrameRateKey: *const AnyObject;
    static AVVideoProfileLevelKey: *const AnyObject;
    static AVVideoProfileLevelH264HighAutoLevel: *const AnyObject;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    pub static kCVPixelBufferPixelFormatTypeKey: *const AnyObject;
    static kCVPixelBufferWidthKey: *const AnyObject;
    static kCVPixelBufferHeightKey: *const AnyObject;
    fn CVPixelBufferPoolCreatePixelBuffer(
        allocator: *const c_void,
        pool: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    pub fn CVPixelBufferLockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
    pub fn CVPixelBufferUnlockBaseAddress(pb: *mut c_void, flags: u64) -> i32;
    pub fn CVPixelBufferGetBaseAddress(pb: *mut c_void) -> *mut u8;
    pub fn CVPixelBufferGetBytesPerRow(pb: *mut c_void) -> usize;
    pub fn CVPixelBufferGetWidth(pb: *mut c_void) -> usize;
    pub fn CVPixelBufferGetHeight(pb: *mut c_void) -> usize;
    pub fn CVPixelBufferRelease(pb: *mut c_void);
}

/// 'BGRA': four bytes a pixel, blue first.
pub const BGRA: u32 = 0x4247_5241;

pub fn class(name: &CStr) -> Result<&'static AnyClass, String> {
    AnyClass::get(name).ok_or_else(|| format!("no {}", name.to_string_lossy()))
}

/// An NSString of `s`.
pub fn nsstring(s: &str) -> Retained<AnyObject> {
    let ns = objc2_foundation::NSString::from_str(s);
    // SAFETY: an NSString is an object.
    unsafe { Retained::cast_unchecked(ns) }
}

/// An NSNumber of `n`.
pub fn nsnumber(n: i64) -> Result<Retained<AnyObject>, String> {
    let c = class(c"NSNumber")?;
    // SAFETY: +numberWithLongLong: answers an autoreleased NSNumber.
    let v: Option<Retained<AnyObject>> = unsafe { msg_send![c, numberWithLongLong: n] };
    v.ok_or_else(|| "no number".to_string())
}

/// An NSDictionary of `pairs`.
pub fn nsdict(pairs: &[(*const AnyObject, &AnyObject)]) -> Result<Retained<AnyObject>, String> {
    let c = class(c"NSMutableDictionary")?;
    // SAFETY: +dictionary answers an autoreleased, empty dictionary; each
    // key is one of the frameworks' constant strings (never null once the
    // framework is loaded) and each value a live object.
    unsafe {
        let d: Option<Retained<AnyObject>> = msg_send![c, dictionary];
        let d = d.ok_or("no dictionary")?;
        for (k, v) in pairs {
            if k.is_null() {
                return Err("a framework's key is missing".to_string());
            }
            let _: () = msg_send![&*d, setObject: *v, forKey: *k];
        }
        Ok(d)
    }
}

/// A file URL of `path`.
pub fn file_url(path: &str) -> Result<Retained<AnyObject>, String> {
    let c = class(c"NSURL")?;
    let p = nsstring(path);
    // SAFETY: +fileURLWithPath: answers an autoreleased NSURL.
    let u: Option<Retained<AnyObject>> = unsafe { msg_send![c, fileURLWithPath: &*p] };
    u.ok_or_else(|| format!("no URL for {path}"))
}

fn describe(err: *mut AnyObject) -> String {
    if err.is_null() {
        return "it failed".to_string();
    }
    // SAFETY: an NSError's localizedDescription is an NSString.
    unsafe {
        let d: Option<Retained<objc2_foundation::NSString>> = msg_send![err, localizedDescription];
        d.map_or("it failed".to_string(), |s| s.to_string())
    }
}

/// A movie being written: H.264 by the hardware encoder, in a QuickTime
/// file written a fragment a second (so one cut short still plays), each
/// frame at the time it showed.
pub struct MovieWriter {
    writer: Retained<AnyObject>,
    input: Retained<AnyObject>,
    adaptor: Retained<AnyObject>,
    pub width: u32,
    pub height: u32,
    last_ms: i64,
    pub path: String,
}

// SAFETY: made and used on one thread only (the capture's writer); the
// struct moves there once.
unsafe impl Send for MovieWriter {}

impl MovieWriter {
    /// A movie begun at `path` for frames of `w × h` pixels (even numbers)
    /// at about `fps`. A file there already is replaced.
    pub fn start(path: &str, w: u32, h: u32, fps: f32) -> Result<MovieWriter, String> {
        let _ = std::fs::remove_file(path);
        let url = file_url(path)?;
        // about 4 bits a pixel a second at 60: a window's text stays sharp
        // (most of a frame is what the one before was), a game's motion fair
        let bitrate = ((w as i64 * h as i64 * 4).clamp(2_000_000, 16_000_000) as f32
            * (fps / 60.0).clamp(0.5, 2.0)) as i64;
        // SAFETY: each message is the documented one for its class, the
        // arguments live objects of the documented kinds; `err` is an out
        // parameter AVFoundation fills or leaves null.
        unsafe {
            let props = nsdict(&[
                (AVVideoAverageBitRateKey, &*nsnumber(bitrate)?),
                (AVVideoMaxKeyFrameIntervalKey, &*nsnumber((fps as i64 * 2).max(2))?),
                (AVVideoExpectedSourceFrameRateKey, &*nsnumber(fps as i64)?),
                (AVVideoProfileLevelKey, &*AVVideoProfileLevelH264HighAutoLevel),
            ])?;
            let settings = nsdict(&[
                (AVVideoCodecKey, &*AVVideoCodecTypeH264),
                (AVVideoWidthKey, &*nsnumber(w as i64)?),
                (AVVideoHeightKey, &*nsnumber(h as i64)?),
                (AVVideoCompressionPropertiesKey, &*props),
            ])?;
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let wc = class(c"AVAssetWriter")?;
            let writer: Option<Retained<AnyObject>> =
                msg_send![wc, assetWriterWithURL: &*url, fileType: AVFileTypeQuickTimeMovie, error: &mut err];
            let writer = writer.ok_or_else(|| format!("the movie {path}: {}", describe(err)))?;
            let ic = class(c"AVAssetWriterInput")?;
            let input: Option<Retained<AnyObject>> = msg_send![ic, assetWriterInputWithMediaType: AVMediaTypeVideo, outputSettings: &*settings];
            let input = input.ok_or("the movie's video input could not be made")?;
            let _: () = msg_send![&*input, setExpectsMediaDataInRealTime: Bool::YES];
            let attrs = nsdict(&[
                (kCVPixelBufferPixelFormatTypeKey, &*nsnumber(BGRA as i64)?),
                (kCVPixelBufferWidthKey, &*nsnumber(w as i64)?),
                (kCVPixelBufferHeightKey, &*nsnumber(h as i64)?),
            ])?;
            let ac = class(c"AVAssetWriterInputPixelBufferAdaptor")?;
            let adaptor: Option<Retained<AnyObject>> = msg_send![ac, assetWriterInputPixelBufferAdaptorWithAssetWriterInput: &*input, sourcePixelBufferAttributes: &*attrs];
            let adaptor = adaptor.ok_or("the movie's frames could not be taken")?;
            let can: Bool = msg_send![&*writer, canAddInput: &*input];
            if !can.as_bool() {
                return Err("the movie does not take this video (its size or codec)".to_string());
            }
            let _: () = msg_send![&*writer, addInput: &*input];
            // written in fragments a second long: a movie cut short (the
            // program ended, the disk filled) plays to its last fragment
            let _: () = msg_send![&*writer, setMovieFragmentInterval: CMTime::ms(1000)];
            let ok: Bool = msg_send![&*writer, startWriting];
            if !ok.as_bool() {
                let e: *mut AnyObject = msg_send![&*writer, error];
                return Err(format!("the movie {path}: {}", describe(e)));
            }
            let _: () = msg_send![&*writer, startSessionAtSourceTime: CMTime::ms(0)];
            Ok(MovieWriter {
                writer,
                input,
                adaptor,
                width: w,
                height: h,
                last_ms: -1,
                path: path.to_string(),
            })
        }
    }

    /// A frame added at `t_ms`: `bgra` its rows, `row` bytes apart, of the
    /// movie's size. `Ok(false)`: the encoder was busy and it is left out.
    pub fn append(&mut self, t_ms: i64, bgra: &[u8], row: usize) -> Result<bool, String> {
        // a movie's times only go forward
        let t = t_ms.max(self.last_ms + 1);
        // SAFETY: the pool is the adaptor's once writing began; the buffer
        // it makes is locked while its bytes are written and released here;
        // the rows copied are within both the source and the buffer.
        unsafe {
            let ready: Bool = msg_send![&*self.input, isReadyForMoreMediaData];
            if !ready.as_bool() {
                return Ok(false);
            }
            let pool: *mut c_void = msg_send![&*self.adaptor, pixelBufferPool];
            if pool.is_null() {
                return Err("the movie has no room for frames (it stopped)".to_string());
            }
            let mut pb: *mut c_void = std::ptr::null_mut();
            if CVPixelBufferPoolCreatePixelBuffer(std::ptr::null(), pool, &mut pb) != 0 || pb.is_null() {
                return Ok(false);
            }
            CVPixelBufferLockBaseAddress(pb, 0);
            let base = CVPixelBufferGetBaseAddress(pb);
            let stride = CVPixelBufferGetBytesPerRow(pb);
            let line = (self.width as usize * 4).min(stride).min(row);
            if !base.is_null() {
                for y in 0..self.height as usize {
                    let from = y * row;
                    if from + line > bgra.len() {
                        break;
                    }
                    std::ptr::copy_nonoverlapping(bgra.as_ptr().add(from), base.add(y * stride), line);
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, 0);
            let ok: Bool = msg_send![&*self.adaptor, appendPixelBuffer: pb, withPresentationTime: CMTime::ms(t)];
            CVPixelBufferRelease(pb);
            if !ok.as_bool() {
                let e: *mut AnyObject = msg_send![&*self.writer, error];
                return Err(format!("the movie: {}", describe(e)));
            }
            self.last_ms = t;
            Ok(true)
        }
    }

    /// The movie ended at `end_ms` and its file finished: its bytes.
    pub fn finish(self, end_ms: i64) -> Result<u64, String> {
        // SAFETY: the documented way to end a writer; `finishWriting`
        // waits for the file.
        unsafe {
            let _: () = msg_send![&*self.input, markAsFinished];
            let _: () = msg_send![&*self.writer, endSessionAtSourceTime: CMTime::ms(end_ms.max(self.last_ms + 1))];
            let ok: Bool = msg_send![&*self.writer, finishWriting];
            if !ok.as_bool() {
                let e: *mut AnyObject = msg_send![&*self.writer, error];
                return Err(format!("the movie: {}", describe(e)));
            }
        }
        Ok(std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0))
    }
}
