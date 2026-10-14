//! cdylib soname setup (D241): the real file libmediaservo_client.so.MAJOR.MINOR.PATCH,
//! soname = libmediaservo_client.so.<MAJOR>. Linux only (macOS uses default dylib naming).

fn main() {
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_else(|_| "0".to_string());
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libmediaservo_client.so.{major}");
    }
}
