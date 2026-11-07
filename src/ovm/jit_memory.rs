//! The JIT's executable memory (OL-442).
//!
//! On macOS the hardened runtime lets a process write machine code only
//! into memory mapped with `MAP_JIT`, and only when the binary carries the
//! `com.apple.security.cs.allow-jit` entitlement. Cranelift's own provider
//! maps plain anonymous pages and flips them to read+execute with
//! `mprotect`, which the hardened runtime answers by killing the process
//! (it needs `allow-unsigned-executable-memory`, a much wider hole). This
//! provider maps code pages with `MAP_JIT` instead and switches the
//! compiling thread between writing and executing them with
//! `pthread_jit_write_protect_np`, so `allow-jit` alone is enough.
//!
//! When `MAP_JIT` is refused (a hardened binary signed without
//! `allow-jit`), [`available`] says so and the JIT stays off: programs run
//! on bytecode instead of dying.
//!
//! Data and read-only sections (constants the compiled code reads) are
//! never executable, so they keep cranelift's system provider.

#[cfg(target_os = "macos")]
use cranelift_jit::{BranchProtection, JITMemoryKind};
use cranelift_jit::{JITMemoryProvider, SystemMemoryProvider};
#[cfg(target_os = "macos")]
use cranelift_module::ModuleResult;
#[cfg(target_os = "macos")]
use std::io;

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    unsafe extern "C" {
        pub fn pthread_jit_write_protect_np(enabled: libc::c_int);
        pub fn pthread_jit_write_protect_supported_np() -> libc::c_int;
        pub fn sys_icache_invalidate(start: *mut c_void, len: libc::size_t);
    }

    /// Map `len` bytes of read+write+execute memory with `MAP_JIT`.
    pub fn map_jit(len: usize) -> std::io::Result<*mut u8> {
        // SAFETY: an anonymous private mapping; the result is checked.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_JIT,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(p as *mut u8)
        }
    }

    /// Let this thread write `MAP_JIT` pages (`true`) or execute them
    /// (`false`). On Intel Macs the pages are always both and this does
    /// nothing.
    pub fn set_writable(writable: bool) {
        // SAFETY: both calls only change this thread's view of MAP_JIT pages.
        unsafe {
            if pthread_jit_write_protect_supported_np() != 0 {
                pthread_jit_write_protect_np(if writable { 0 } else { 1 });
            }
        }
    }
}

/// Whether the JIT may run in this process: on macOS, whether the kernel
/// grants a `MAP_JIT` mapping (it refuses one to a hardened binary signed
/// without `allow-jit`). Probed once. `OLANG_NO_JIT=1` also answers no.
pub fn available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        if std::env::var_os("OLANG_NO_JIT").is_some_and(|v| !v.is_empty() && v != "0") {
            return false;
        }
        probe()
    })
}

#[cfg(target_os = "macos")]
fn probe() -> bool {
    let page = page_size();
    match mac::map_jit(page) {
        Ok(p) => {
            // SAFETY: the page just mapped, unmapped once.
            unsafe { libc::munmap(p as *mut libc::c_void, page) };
            true
        }
        Err(e) => {
            if std::env::var_os("OLANG_JIT_DEBUG").is_some() {
                eprintln!("[jit] MAP_JIT refused ({e}); the JIT is off, bytecode runs instead");
            }
            false
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn probe() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn page_size() -> usize {
    // SAFETY: sysconf has no preconditions.
    let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if n > 0 { n as usize } else { 16384 }
}

/// The provider handed to every `JITBuilder`.
pub fn provider() -> Box<dyn JITMemoryProvider + Send> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MapJitProvider::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(SystemMemoryProvider::new())
    }
}

/// Ends a compile's write window on every path out of it, a failed
/// compile included: the compiling thread goes back to executing
/// `MAP_JIT` pages. A no-op off macOS.
pub struct WriteWindow;

impl WriteWindow {
    pub fn open() -> Self {
        WriteWindow
    }
}

impl Drop for WriteWindow {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        mac::set_writable(false);
    }
}

/// One `MAP_JIT` region: the code pages of one or more functions.
#[cfg(target_os = "macos")]
struct Region {
    ptr: *mut u8,
    len: usize,
}

#[cfg(target_os = "macos")]
pub struct MapJitProvider {
    /// Regions already sealed (executable, icache flushed).
    sealed: Vec<Region>,
    /// The region being filled, and how far.
    current: Option<Region>,
    position: usize,
    /// Regions filled since the last finalize.
    pending: Vec<Region>,
    /// Data and read-only sections.
    data: SystemMemoryProvider,
}

// SAFETY: the raw pointers are addresses of mappings this provider owns;
// the owning JitCache is used from one thread at a time.
#[cfg(target_os = "macos")]
unsafe impl Send for MapJitProvider {}

#[cfg(target_os = "macos")]
impl MapJitProvider {
    fn new() -> Self {
        MapJitProvider {
            sealed: Vec::new(),
            current: None,
            position: 0,
            pending: Vec::new(),
            data: SystemMemoryProvider::new(),
        }
    }

    fn allocate_code(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
        let align = usize::try_from(align).unwrap_or(16).max(1);
        // Writing code pages needs this thread in write mode until the
        // compile's finalize (or its WriteWindow) flips it back.
        mac::set_writable(true);
        if let Some(cur) = &self.current {
            let start = self.position.div_ceil(align) * align;
            if start + size <= cur.len {
                self.position = start + size;
                // SAFETY: within the region just checked.
                return Ok(unsafe { cur.ptr.add(start) });
            }
        }
        if let Some(full) = self.current.take() {
            self.pending.push(full);
        }
        // At least 64 KiB per mapping: compiles are small and frequent.
        let page = page_size();
        let len = size.max(64 * 1024).div_ceil(page) * page;
        let ptr = mac::map_jit(len)?;
        self.current = Some(Region { ptr, len });
        self.position = size;
        Ok(ptr)
    }
}

#[cfg(target_os = "macos")]
impl JITMemoryProvider for MapJitProvider {
    fn allocate(&mut self, size: usize, align: u64, kind: JITMemoryKind) -> io::Result<*mut u8> {
        match kind {
            JITMemoryKind::Executable => self.allocate_code(size, align),
            other => self.data.allocate(size, align, other),
        }
    }

    unsafe fn free_memory(&mut self) {
        for r in self
            .sealed
            .drain(..)
            .chain(self.pending.drain(..))
            .chain(self.current.take())
        {
            // SAFETY: each region is a live mapping this provider made.
            unsafe { libc::munmap(r.ptr as *mut libc::c_void, r.len) };
        }
        self.position = 0;
        // SAFETY: forwarded; the caller guarantees nothing runs the code.
        unsafe { self.data.free_memory() };
    }

    fn finalize(&mut self, branch_protection: BranchProtection) -> ModuleResult<()> {
        self.data.finalize(branch_protection)?;
        // Every relocation is written: back to execute mode, then make the
        // instruction cache see the new code. The current region stays
        // open for the next compile (its unwritten tail is never run), so
        // it is flushed but not retired.
        mac::set_writable(false);
        for r in &self.pending {
            // SAFETY: a live mapping of r.len bytes.
            unsafe { mac::sys_icache_invalidate(r.ptr as *mut std::ffi::c_void, r.len) };
        }
        if let Some(cur) = &self.current {
            // SAFETY: the written prefix of a live mapping.
            unsafe { mac::sys_icache_invalidate(cur.ptr as *mut std::ffi::c_void, self.position) };
        }
        self.sealed.append(&mut self.pending);
        Ok(())
    }
}
