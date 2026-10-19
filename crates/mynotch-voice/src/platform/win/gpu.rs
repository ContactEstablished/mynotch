//! Which GPUs whisper.cpp's backends found. This is ggml's C API rather than Win32, but it's
//! `unsafe` FFI, so it lives here with the rest.

use std::ffi::CStr;

use whisper_rs::whisper_rs_sys::{
    ggml_backend_dev_count, ggml_backend_dev_description, ggml_backend_dev_get, ggml_backend_dev_type,
    ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU, ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU,
};

/// The GPU devices ggml can use ("NVIDIA GeForce RTX 4080 SUPER"); empty if CUDA found none,
/// in which case whisper.cpp would quietly run on the CPU.
pub fn devices() -> Vec<String> {
    unsafe {
        (0..ggml_backend_dev_count())
            .map(|i| ggml_backend_dev_get(i))
            .filter(|&dev| {
                let kind = ggml_backend_dev_type(dev);
                kind == ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                    || kind == ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU
            })
            .map(|dev| {
                let name = ggml_backend_dev_description(dev);
                if name.is_null() { String::new() } else { CStr::from_ptr(name).to_string_lossy().into_owned() }
            })
            .collect()
    }
}
