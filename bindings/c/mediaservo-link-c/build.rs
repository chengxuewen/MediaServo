//! cdylib soname setup (D241): the real file libmediaservo_link.so.MAJOR.MINOR.PATCH,
//! soname = libmediaservo_link.so.<MAJOR>. Linux only (macOS uses default dylib naming).

fn main() {
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_else(|_| "0".to_string());
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libmediaservo_link.so.{major}");
        // doc25 §5 T2+T1 — per-so export whitelist + vendored C++/GCC runtime.
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
        println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script,{dir}/mediaservo-link.map");
        // E1 empirical: conda gcc's -static-libstdc++ alone is a NO-OP (undefined operator new).
        // Explicit -Bstatic sandwich binds libstdc++.a (libgcc_s stays dynamic, stock-focal-safe GCC_4.2.0 nodes).
        println!("cargo:rustc-cdylib-link-arg=-Wl,-Bstatic,-lstdc++,-Bdynamic");
        println!("cargo:rustc-cdylib-link-arg=-Wl,--exclude-libs,ALL");
    }
}
