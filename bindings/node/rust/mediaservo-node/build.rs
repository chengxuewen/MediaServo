fn main() {
    napi_build::setup();
    // Complete the FFmpeg shared-library linkage (same NEEDED alignment as deck-c — the ffmpeg-the-third
    // flag does not propagate fully across the crate chain; the napi artifact once linked only libavdevice, causing "Protocol not found").
    // pixi environment: PIXI_PROJECT_ROOT/.pixi/envs/default/lib (conda FFmpeg 9.0)
    if let Ok(root) = std::env::var("PIXI_PROJECT_ROOT") {
        let lib_dir = std::path::Path::new(&root).join(".pixi/envs/default/lib");
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            for lib in ["avformat", "avcodec", "avutil", "avdevice", "swscale", "swresample"] {
                println!("cargo:rustc-link-lib=dylib={lib}");
            }
        }
    }
    // V2 delivery-surface fix (confirmed by the 09-17 drill: CXXABI_1.3.15 not found): the build machine's conda gcc14
    // libstdc++ must not enter the .node dynamically — clean systems (ubuntu-22.04 with GCC11 runtime) refuse to load it.
    // Static-link stdc++/gcc (the napi surface is thin; trade size for portability — same philosophy as the node-gyp precedent).
    #[cfg(target_os = "linux")]
    {
        // doc25 T1 probe (validated on client-c 2026-10-15): conda -static-libstdc++ is a silent no-op;
        // bind libstdc++.a via the compiler-reported absolute path, plus exclude-libs so the vendored
        // std symbols stay LOCAL inside the .node (no interposition on node's own runtime).
        let a_path = std::env::var("CXX").or_else(|_| std::env::var("CC")).ok().and_then(|c| {
            std::process::Command::new(c).arg("-print-file-name=libstdc++.a").output().ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|p| p.ends_with(".a") && std::path::Path::new(p).exists())
        });
        if let Some(a) = a_path {
            if let Some(d) = std::path::Path::new(&a).parent() {
                println!("cargo:rustc-link-search=native={}", d.display());
            }
            println!("cargo:rustc-link-lib=static=stdc++");
        } else {
            println!("cargo:rustc-link-arg=-Wl,-Bstatic,-lstdc++,-Bdynamic");
        }
        println!("cargo:rustc-cdylib-link-arg=-Wl,--exclude-libs,ALL");
    }
}
