//! Safe wrapper around libfaust JIT compilation.

use std::ffi::CString;

use crate::ffi;
use crate::ui_visitor::{self, FaustParam};

/// A compiled Faust DSP (owns factory + instance).
pub struct CompiledDsp {
    factory: *mut ffi::LlvmDspFactory,
    instance: *mut ffi::LlvmDsp,
    pub num_inputs: usize,
    pub num_outputs: usize,
    pub params: Vec<FaustParam>,
}

// Safety: raw pointers are only accessed from one thread after transfer.
unsafe impl Send for CompiledDsp {}

impl CompiledDsp {
    pub fn instance(&self) -> *mut ffi::LlvmDsp { self.instance }
    pub fn params(&self) -> &[FaustParam] { &self.params }
}

impl Drop for CompiledDsp {
    fn drop(&mut self) {
        unsafe {
            ffi::deleteCDSPInstance(self.instance);
            ffi::deleteCDSPFactory(self.factory);
        }
    }
}

/// Default Faust library search path.
const DEFAULT_LIB_PATH: &str = "/usr/local/share/faust";

/// Compile a Faust DSP from source code.
pub fn compile(name: &str, source: &str, lib_path: Option<&str>) -> Result<CompiledDsp, String> {
    let c_name = CString::new(name).map_err(|e| e.to_string())?;
    let c_source = CString::new(source).map_err(|e| e.to_string())?;
    let c_target = CString::new("").unwrap();

    let lib = lib_path.unwrap_or(DEFAULT_LIB_PATH);
    let include_flag = CString::new(format!("-I{lib}")).unwrap();
    let argv = [include_flag.as_ptr()];

    let mut error_buf = vec![0u8; 4096];

    let factory = unsafe {
        ffi::createCDSPFactoryFromString(
            c_name.as_ptr(),
            c_source.as_ptr(),
            argv.len() as i32,
            argv.as_ptr(),
            c_target.as_ptr(),
            error_buf.as_mut_ptr() as *mut i8,
            -1,
        )
    };

    if factory.is_null() {
        let msg = extract_error(&error_buf);
        return Err(msg);
    }

    let instance = unsafe { ffi::createCDSPInstance(factory) };
    if instance.is_null() {
        unsafe { ffi::deleteCDSPFactory(factory); }
        return Err("Failed to create DSP instance".into());
    }

    let num_inputs = unsafe { ffi::getNumInputsCDSPInstance(instance) } as usize;
    let num_outputs = unsafe { ffi::getNumOutputsCDSPInstance(instance) } as usize;

    let mut params = Vec::new();
    let mut glue = ui_visitor::build_ui_glue(&mut params);
    unsafe { ffi::buildUserInterfaceCDSPInstance(instance, &mut glue); }

    Ok(CompiledDsp { factory, instance, num_inputs, num_outputs, params })
}

/// Compile a Faust DSP from a .dsp file path.
pub fn compile_file(name: &str, path: &str, lib_path: Option<&str>) -> Result<CompiledDsp, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    compile(name, &source, lib_path)
}

fn extract_error(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}
