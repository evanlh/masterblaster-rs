//! Machine-code disk cache for libfaust factories.
//!
//! Writing a factory to native machine code via `writeCDSPFactoryToMachineFile`
//! lets us skip LLVM JIT compilation on subsequent runs: loading a `.machine`
//! file takes <1ms versus ~500–800ms to JIT a complex DSP like WahPro2.
//!
//! Cache entries are keyed by a hash of the DSP source *and* the libfaust
//! version, so they are invalidated automatically when either changes.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;

use crate::ffi;

/// Platform cache directory for machine-code files (e.g. `~/Library/Caches`).
fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("masterblaster").join("faust"))
}

/// libfaust version string — hashed into the key to survive library upgrades.
fn faust_version() -> String {
    unsafe {
        let ptr = ffi::getCLibFaustVersion();
        if ptr.is_null() {
            return String::new();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

/// Cache file path: `{name}-{md5}.machine`, hashed over source + libfaust version.
fn cache_path(name: &str, source: &str) -> Option<PathBuf> {
    let dir = cache_dir()?;
    let key = format!("{source}\0{}", faust_version());
    let digest = md5::compute(key.as_bytes());
    Some(dir.join(format!("{name}-{digest:x}.machine")))
}

/// Try to load a factory from the disk cache. Returns `None` on miss or failure.
pub fn try_load(name: &str, source: &str) -> Option<*mut ffi::LlvmDspFactory> {
    let path = cache_path(name, source)?;
    if !path.is_file() {
        return None;
    }
    let c_path = CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let c_target = CString::new("").unwrap();
    let mut error_buf = vec![0u8; 4096];
    let factory = unsafe {
        ffi::readCDSPFactoryFromMachineFile(
            c_path.as_ptr(),
            c_target.as_ptr(),
            error_buf.as_mut_ptr() as *mut c_char,
        )
    };
    if factory.is_null() {
        None
    } else {
        Some(factory)
    }
}

/// Write a factory to the disk cache. Best-effort: failures are ignored so a
/// read-only or absent cache directory only costs the JIT on the next run.
pub fn save(name: &str, source: &str, factory: *mut ffi::LlvmDspFactory) {
    let Some(dir) = cache_dir() else { return };
    let Some(path) = cache_path(name, source) else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(c_path) = CString::new(path.to_string_lossy().as_bytes()) else { return };
    let c_target = CString::new("").unwrap();
    unsafe {
        ffi::writeCDSPFactoryToMachineFile(factory, c_path.as_ptr(), c_target.as_ptr());
    }
}