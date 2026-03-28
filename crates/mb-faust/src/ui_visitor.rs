//! Parameter discovery via UIGlue callbacks.

use std::ffi::CStr;
use std::os::raw::{c_char, c_float, c_void};

use crate::ffi::{Soundfile, UIGlue};

/// A discovered Faust parameter.
pub struct FaustParam {
    pub label: String,
    pub zone: *mut f32,
    pub init: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

/// Build a UIGlue that collects parameters into a `Vec<FaustParam>`.
pub fn build_ui_glue(params: &mut Vec<FaustParam>) -> UIGlue {
    UIGlue {
        ui_interface: params as *mut Vec<FaustParam> as *mut c_void,
        open_tab_box: Some(noop_box),
        open_horizontal_box: Some(noop_box),
        open_vertical_box: Some(noop_box),
        close_box: Some(noop_close),
        add_button: Some(cb_button),
        add_check_button: Some(cb_button),
        add_vertical_slider: Some(cb_slider),
        add_horizontal_slider: Some(cb_slider),
        add_num_entry: Some(cb_slider),
        add_horizontal_bargraph: Some(noop_bargraph),
        add_vertical_bargraph: Some(noop_bargraph),
        add_soundfile: Some(noop_soundfile),
        declare: Some(noop_declare),
    }
}

unsafe fn label_to_string(label: *const c_char) -> String {
    if label.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(label) }.to_string_lossy().into_owned()
}

unsafe extern "C" fn noop_box(_ui: *mut c_void, _label: *const c_char) {}
unsafe extern "C" fn noop_close(_ui: *mut c_void) {}

unsafe extern "C" fn noop_bargraph(
    _ui: *mut c_void, _label: *const c_char, _zone: *mut c_float, _min: c_float, _max: c_float,
) {}

unsafe extern "C" fn noop_soundfile(
    _ui: *mut c_void, _label: *const c_char, _url: *const c_char, _sf: *mut *mut Soundfile,
) {}

unsafe extern "C" fn noop_declare(
    _ui: *mut c_void, _zone: *mut c_float, _key: *const c_char, _value: *const c_char,
) {}

unsafe extern "C" fn cb_button(ui: *mut c_void, label: *const c_char, zone: *mut c_float) {
    let params = unsafe { &mut *(ui as *mut Vec<FaustParam>) };
    params.push(FaustParam {
        label: unsafe { label_to_string(label) },
        zone,
        init: 0.0,
        min: 0.0,
        max: 1.0,
        step: 1.0,
    });
}

unsafe extern "C" fn cb_slider(
    ui: *mut c_void,
    label: *const c_char,
    zone: *mut c_float,
    init: c_float,
    min: c_float,
    max: c_float,
    step: c_float,
) {
    let params = unsafe { &mut *(ui as *mut Vec<FaustParam>) };
    params.push(FaustParam {
        label: unsafe { label_to_string(label) },
        zone,
        init,
        min,
        max,
        step,
    });
}

/// Convert FaustParam list to ParamInfo list for the Machine trait.
/// Maps f32 range to i32 with 65535 resolution.
pub fn to_param_infos(params: &[FaustParam]) -> Vec<mb_engine::machine::ParamInfo> {
    params.iter().enumerate().map(|(i, p)| {
        let scale = if (p.max - p.min).abs() > f32::EPSILON { 65535.0 / (p.max - p.min) } else { 1.0 };
        let default = ((p.init - p.min) * scale) as i32;
        mb_engine::machine::ParamInfo::new(i as u16, &p.label, 0, 65535, default)
    }).collect()
}

/// Convert a scaled i32 param value back to f32 for a given FaustParam.
pub fn scaled_to_f32(param: &FaustParam, value: i32) -> f32 {
    let t = value as f32 / 65535.0;
    param.min + t * (param.max - param.min)
}
