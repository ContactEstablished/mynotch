//! whisper.cpp's CUDA backend (in mynotch-voice) imports cuBLAS from its DLL. Loaded at
//! startup, a missing DLL would stop the whole notch from starting, so it's delay-loaded
//! instead: the notch starts without it, and voice checks for it (`platform::win::CUDA_DLLS`)
//! before it touches whisper.cpp, turning itself off if it's missing. The CUDA runtime is
//! linked statically and finds the driver itself, so cuBLAS is the only CUDA import.

/// CUDA 13's cuBLAS. Another CUDA version names it differently, and delay-loading the wrong
/// name would quietly leave the real one a startup import: hence the check below.
const CUBLAS: &str = "cublas64_13.dll";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");
    if let Some(cuda) = std::env::var_os("CUDA_PATH") {
        let bin = std::path::Path::new(&cuda).join("bin");
        assert!(
            bin.join("x64").join(CUBLAS).exists() || bin.join(CUBLAS).exists(),
            "CUDA_PATH ({}) isn't CUDA 13: update CUBLAS in build.rs and CUDA_DLLS in mynotch-voice's platform::win",
            bin.display()
        );
    }
    println!("cargo:rustc-link-arg=/DELAYLOAD:{CUBLAS}");
    println!("cargo:rustc-link-arg=delayimp.lib");
}
