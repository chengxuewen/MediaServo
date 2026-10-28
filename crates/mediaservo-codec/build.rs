//! Build script for mediaservo-codec.
//!
//! # FFmpeg backend (feature = "backend-ffmpeg")
//! 1. Check pkg-config for system FFmpeg (apt/brew)
//! 2. Check FFMPEG_DIR env var for pre-built static libs
//! 3. If neither found, print warning (crate compiles but FFmpeg backend is stub)

fn main() {
    #[cfg(feature = "backend-ffmpeg")]
    detect_ffmpeg();
}

#[cfg(feature = "backend-ffmpeg")]
fn detect_ffmpeg() {
    // Try pkg-config first (dynamic link, dev convenience)
    let pkgs = &["libavcodec", "libavutil", "libavformat"];
    let found_all = pkgs.iter().all(|p| pkg_config::probe_library(p).is_ok());

    if found_all {
        println!("cargo:rustc-cfg=feature=\"ffmpeg-system\"");
        return;
    }

    // Try FFMPEG_DIR for pre-built static libs
    if let Ok(dir) = std::env::var("FFMPEG_DIR") {
        let lib_dir = format!("{}/lib", dir);
        println!("cargo:rustc-link-search=native={}", lib_dir);
        for lib in &["avcodec", "avutil", "avformat"] {
            println!("cargo:rustc-link-lib=static={}", lib);
        }
        println!("cargo:rustc-cfg=feature=\"ffmpeg-static\"");
        return;
    }

    // No FFmpeg found — backend is stub. Print guidance.
    println!("cargo:warning=FFmpeg not found. Install libavcodec-dev or set FFMPEG_DIR.");
    println!(
        "cargo:warning=  Ubuntu: apt install libavcodec-dev libavutil-dev libavformat-dev libswscale-dev libclang-dev"
    );
    println!("cargo:warning=  macOS:  brew install ffmpeg");
    println!("cargo:warning=FFmpeg backend will be a stub (no real encoding/decoding).");
}
