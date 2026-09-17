fn main() {
    napi_build::setup();
    // FFmpeg 动态库链接补齐（deck-c 同款 NEEDED 对齐——ffmpeg-the-third 标志在
    // 跨 crate 链上传播不完整，napi 产物曾仅 libavdevice 导致 Protocol not found）。
    // pixi 环境: PIXI_PROJECT_ROOT/.pixi/envs/default/lib（conda FFmpeg 9.0）
    if let Ok(root) = std::env::var("PIXI_PROJECT_ROOT") {
        let lib_dir = std::path::Path::new(&root).join(".pixi/envs/default/lib");
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            for lib in ["avformat", "avcodec", "avutil", "avdevice", "swscale", "swresample"] {
                println!("cargo:rustc-link-lib=dylib={lib}");
            }
        }
    }
    // V2 交付面修复（09-17 演练实锤 CXXABI_1.3.15 not found）：构建机 conda gcc14 的
    // libstdc++ 不得以动态形式进入 .node——干净系统（ubuntu-22.04 GCC11 运行时）直接拒载。
    // 静态化 stdc++/gcc（napi 面薄，体积换可移植；node-gyp 先例同哲学）。
    println!("cargo:rustc-link-arg=-static-libstdc++");
    println!("cargo:rustc-link-arg=-static-libgcc");
}
