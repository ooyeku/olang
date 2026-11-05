//! Code layers (OL-445): a macOS application can boot a newer program
//! than the one sealed inside it, from a folder outside the bundle that
//! an updater writes, without breaking the bundle's signature and without
//! running anything its publisher did not sign.
//!
//! The application's Info.plist names the folder, `OlangLayersDirectory`
//! (absolute, or starting `~/`), and the public key, `OlangLayersPublicKey`:
//! a file in `Contents/Resources` (default `layers.pub`) holding the
//! 32-byte Ed25519 key in base64. Both are inside the signed bundle. The
//! folder holds a state file and one folder per layer:
//!
//! ```text
//! <layers>/state.json        {"active": "1.4.0", "tries": 0, "max_tries": 3}
//! <layers>/1.4.0/app.olb     the program (`olang build --payload`)
//! <layers>/1.4.0/app.olb.sig its Ed25519 signature, base64
//! ```
//!
//! At start the runtime reads the state, and boots the active layer only
//! when its signature verifies with the bundle's key, its payload was
//! built for this runtime (OL-443), and it has been tried fewer than
//! `max_tries` times (default 3). It counts the try in the state file
//! before the program starts; the program, when it is satisfied the
//! layer works, sets `tries` back to 0 itself. Anything else, a layer
//! that fails a check or has used its tries, boots the bundled
//! `Contents/Resources/app.olb`, and `runtime.code_layer()` says why.
//! Which layer is active, when to give up on one, and what to do then
//! are the program's (a library's) policy; the runtime only refuses to
//! run what it cannot trust and counts.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How the application booted, for `runtime.code_layer()`.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// The layers folder the Info.plist names.
    pub layers: Option<PathBuf>,
    /// The layer the state file names.
    pub active: Option<String>,
    /// The layer running now; `None` when the bundled program runs.
    pub running: Option<String>,
    /// The tries counted, this one included when a layer runs.
    pub tries: u32,
    pub max_tries: u32,
    /// Why the active layer is not the one running.
    pub fallback: Option<String>,
}

static STATUS: OnceLock<Status> = OnceLock::new();

/// How this application booted; `None` outside an application that
/// names a layers folder.
pub fn status() -> Option<&'static Status> {
    STATUS.get()
}

/// The default number of tries a layer gets.
pub const DEFAULT_TRIES: u32 = 3;

#[derive(serde::Deserialize, serde::Serialize, Default)]
struct State {
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    tries: u32,
    #[serde(default)]
    max_tries: Option<u32>,
    /// Whatever else the library keeps there, kept as it was.
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

/// The `.app` an executable at `Contents/MacOS/<name>` belongs to.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// The program to boot from a code layer, its bytes already verified,
/// when the application names a layers folder and its active layer
/// passes every check. `runtime_ok` checks a payload was built for this
/// runtime. Records the outcome for [`status`] either way.
pub fn choose(exe: &Path, runtime_ok: impl Fn(&[u8]) -> Result<(), String>) -> Option<Vec<u8>> {
    let bundle = bundle_of(exe)?;
    let dir = info_string("OlangLayersDirectory")?;
    let dir = expand(&dir)?;
    let mut status = Status {
        layers: Some(dir.clone()),
        max_tries: DEFAULT_TRIES,
        ..Status::default()
    };
    let outcome = pick(&bundle, &dir, &mut status, runtime_ok);
    if let Err(why) = &outcome {
        status.running = None;
        status.fallback = Some(why.clone());
    }
    let _ = STATUS.set(status);
    outcome.ok().flatten()
}

fn pick(
    bundle: &Path,
    dir: &Path,
    status: &mut Status,
    runtime_ok: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    let state_path = dir.join("state.json");
    let mut state: State = match std::fs::read(&state_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} does not read: {e}", state_path.display()))?,
        // no state: no layer chosen yet, the bundled program runs
        Err(_) => return Ok(None),
    };
    let max = state.max_tries.unwrap_or(DEFAULT_TRIES).max(1);
    status.max_tries = max;
    status.tries = state.tries;
    let Some(active) = state.active.clone().filter(|a| !a.is_empty()) else {
        return Ok(None);
    };
    status.active = Some(active.clone());
    if active.contains('/') || active.contains('\\') || active == "." || active == ".." {
        return Err(format!("the active layer \"{active}\" is not a folder's name"));
    }
    if state.tries >= max {
        return Err(format!(
            "layer {active} was tried {} times without the program marking it good (max_tries {max})",
            state.tries
        ));
    }
    let key = public_key(bundle)?;
    let layer = dir.join(&active);
    let program = std::fs::read(layer.join("app.olb"))
        .map_err(|e| format!("layer {active}: app.olb: {e}"))?;
    let sig_text = std::fs::read_to_string(layer.join("app.olb.sig"))
        .map_err(|e| format!("layer {active}: app.olb.sig: {e}"))?;
    let sig = decode_b64::<64>(sig_text.trim())
        .ok_or_else(|| format!("layer {active}: app.olb.sig is not a base64 Ed25519 signature"))?;
    if !crate::stdlib::crypto::ed25519_verify_raw(&key, &sig, &program) {
        return Err(format!(
            "layer {active}: its signature does not verify with the application's key"
        ));
    }
    runtime_ok(&program).map_err(|why| format!("layer {active}: {why}"))?;
    // the try is counted before the program starts: one that dies on
    // its way up has used it
    state.tries += 1;
    write_state(&state_path, &state)
        .map_err(|e| format!("layer {active}: cannot count the try in {}: {e}", state_path.display()))?;
    status.tries = state.tries;
    status.running = Some(active);
    Ok(Some(program))
}

/// Replace the state file whole (a temporary beside it, renamed).
fn write_state(path: &Path, state: &State) -> std::io::Result<()> {
    let text = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// The bundle's layer key: `Contents/Resources/<OlangLayersPublicKey>`
/// (default `layers.pub`), 32 bytes in base64.
fn public_key(bundle: &Path) -> Result<[u8; 32], String> {
    let name = info_string("OlangLayersPublicKey").unwrap_or_else(|| "layers.pub".to_string());
    if name.contains('/') || name == ".." {
        return Err(format!("OlangLayersPublicKey \"{name}\" is not a file in Contents/Resources"));
    }
    let path = bundle.join("Contents").join("Resources").join(&name);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("the layers' public key {}: {e}", path.display()))?;
    decode_b64::<32>(text.trim())
        .ok_or_else(|| format!("{} is not a base64 Ed25519 public key", path.display()))
}

fn decode_b64<const N: usize>(text: &str) -> Option<[u8; N]> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()?
        .try_into()
        .ok()
}

fn expand(dir: &str) -> Option<PathBuf> {
    if let Some(rest) = dir.strip_prefix("~/") {
        return std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest));
    }
    let p = PathBuf::from(dir);
    p.is_absolute().then_some(p)
}

/// A string from the main bundle's Info.plist (CoreFoundation reads XML
/// and binary plists alike).
#[cfg(target_os = "macos")]
fn info_string(key: &str) -> Option<String> {
    use std::ffi::{CString, c_char, c_void};
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetMainBundle() -> *const c_void;
        fn CFBundleGetValueForInfoDictionaryKey(b: *const c_void, key: *const c_void) -> *const c_void;
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, enc: u32) -> *const c_void;
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, enc: u32) -> u8;
        fn CFGetTypeID(v: *const c_void) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFRelease(v: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    let ckey = CString::new(key).ok()?;
    // SAFETY: CoreFoundation's documented calls; the key string is made
    // and released here, the value is the bundle's (not ours to release).
    unsafe {
        let bundle = CFBundleGetMainBundle();
        if bundle.is_null() {
            return None;
        }
        let k = CFStringCreateWithCString(std::ptr::null(), ckey.as_ptr(), UTF8);
        if k.is_null() {
            return None;
        }
        let v = CFBundleGetValueForInfoDictionaryKey(bundle, k);
        CFRelease(k);
        if v.is_null() || CFGetTypeID(v) != CFStringGetTypeID() {
            return None;
        }
        let mut buf = vec![0 as c_char; 4096];
        if CFStringGetCString(v, buf.as_mut_ptr(), buf.len() as isize, UTF8) == 0 {
            return None;
        }
        Some(
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

#[cfg(not(target_os = "macos"))]
fn info_string(_key: &str) -> Option<String> {
    None
}
