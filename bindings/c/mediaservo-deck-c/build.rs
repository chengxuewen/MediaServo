//! cdylib soname setup (D241): the real file libmediaservo_deck.so.MAJOR.MINOR.PATCH,
//! soname = libmediaservo_deck.so.<MAJOR>. Linux only (macOS uses default dylib naming).
//!
//! R10 (binding-review-plan-v2): deck-c statically links FFmpeg (built from the ffmpeg-the-third
//! source), and `--exclude-libs,ALL` keeps static-library symbols out of the dynamic symbol table — when a host binary
//! loads deck.so together with another FFmpeg version there are no symbol clashes (this crate's ms_deck_* exports
//! are defined in the cdylib's own object files and are unaffected).

fn main() {
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_else(|_| "0".to_string());
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libmediaservo_deck.so.{major}");
        println!("cargo:rustc-cdylib-link-arg=-Wl,--exclude-libs,ALL");
    }
}
