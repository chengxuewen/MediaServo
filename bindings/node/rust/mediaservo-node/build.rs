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
    println!("cargo:rustc-link-arg=-static-libstdc++");
    println!("cargo:rustc-link-arg=-static-libgcc");
}
