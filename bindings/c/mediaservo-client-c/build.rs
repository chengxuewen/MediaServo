//! cdylib soname setup (D241): the real file libmediaservo_client.so.MAJOR.MINOR.PATCH,
//! soname = libmediaservo_client.so.<MAJOR>. Linux only (macOS uses default dylib naming).

fn main() {
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_else(|_| "0".to_string());
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libmediaservo_client.so.{major}");
        // doc25 §5 T2+T1 — per-so export whitelist + vendored C++/GCC runtime (node build.rs:18 / deck-c R10 precedent).
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
        println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script,{dir}/mediaservo-client.map");
        // doc25 T1: conda's -static-libstdc++ is a silent NO-OP (E1 empirical); the cc crates' plain
        // -lstdc++ binds the DSO and leaves versioned UND (the GLIBCXX_3.4.30 wall). Ask the active
        // compiler for libstdc++.a's absolute path and bind it statically; fall back to the explicit
        // -Bstatic sandwich (and keep exclude-libs regardless).
        let a_path = std::env::var("CXX").or_else(|_| std::env::var("CC")).ok().and_then(|c| {
            std::process::Command::new(c).arg("-print-file-name=libstdc++.a").output().ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|p| p.ends_with(".a") && std::path::Path::new(p).exists())
        });
        if let Some(a) = a_path {
            if let Some(dir) = std::path::Path::new(&a).parent() {
                println!("cargo:rustc-link-search=native={}", dir.display());
            }
            println!("cargo:rustc-link-lib=static=stdc++");
        } else {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-Bstatic,-lstdc++,-Bdynamic");
        }
        println!("cargo:rustc-cdylib-link-arg=-Wl,--exclude-libs,ALL");
    }
}
