//! Raw extern "C" bindings to libfaust's C API.

use std::os::raw::{c_char, c_float, c_int, c_void};

// Opaque types
#[repr(C)]
pub struct LlvmDspFactory {
    _private: [u8; 0],
}

#[repr(C)]
pub struct LlvmDsp {
    _private: [u8; 0],
}

// Soundfile opaque (unused but needed for UIGlue layout)
#[repr(C)]
pub struct Soundfile {
    _private: [u8; 0],
}

/// UIGlue — matches CInterface.h exactly (14 fields).
#[repr(C)]
pub struct UIGlue {
    pub ui_interface: *mut c_void,

    // Layout widgets
    pub open_tab_box: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    pub open_horizontal_box: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    pub open_vertical_box: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    pub close_box: Option<unsafe extern "C" fn(*mut c_void)>,

    // Active widgets
    pub add_button: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float)>,
    pub add_check_button: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float)>,
    pub add_vertical_slider: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float, c_float, c_float, c_float, c_float),
    >,
    pub add_horizontal_slider: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float, c_float, c_float, c_float, c_float),
    >,
    pub add_num_entry: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float, c_float, c_float, c_float, c_float),
    >,

    // Passive widgets
    pub add_horizontal_bargraph: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float, c_float, c_float),
    >,
    pub add_vertical_bargraph: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float, c_float, c_float),
    >,

    // Soundfile
    pub add_soundfile: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut *mut Soundfile),
    >,

    // Declare
    pub declare: Option<unsafe extern "C" fn(*mut c_void, *mut c_float, *const c_char, *const c_char)>,
}

/// MetaGlue — matches CInterface.h.
#[repr(C)]
pub struct MetaGlue {
    pub meta_interface: *mut c_void,
    pub declare: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char)>,
}

extern "C" {
    pub fn createCDSPFactoryFromString(
        name_app: *const c_char,
        dsp_content: *const c_char,
        argc: c_int,
        argv: *const *const c_char,
        target: *const c_char,
        error_msg: *mut c_char,
        opt_level: c_int,
    ) -> *mut LlvmDspFactory;

    pub fn deleteCDSPFactory(factory: *mut LlvmDspFactory) -> bool;

    pub fn createCDSPInstance(factory: *mut LlvmDspFactory) -> *mut LlvmDsp;
    pub fn deleteCDSPInstance(dsp: *mut LlvmDsp);

    pub fn initCDSPInstance(dsp: *mut LlvmDsp, sample_rate: c_int);
    pub fn instanceClearCDSPInstance(dsp: *mut LlvmDsp);

    pub fn getNumInputsCDSPInstance(dsp: *mut LlvmDsp) -> c_int;
    pub fn getNumOutputsCDSPInstance(dsp: *mut LlvmDsp) -> c_int;

    pub fn buildUserInterfaceCDSPInstance(dsp: *mut LlvmDsp, ui: *mut UIGlue);

    pub fn computeCDSPInstance(
        dsp: *mut LlvmDsp,
        count: c_int,
        inputs: *const *const c_float,
        outputs: *mut *mut c_float,
    );

    pub fn metadataCDSPInstance(dsp: *mut LlvmDsp, meta: *mut MetaGlue);

    pub fn freeCMemory(ptr: *mut c_void);
}
