# FFmpeg Static Build & Linking Strategy

> **Decision ref**: D71 — GStreamer (Field/Host) + FFmpeg (Remote), unified VideoDecoder/VideoEncoder traits
> **Goal**: a zero-runtime-dependency statically linked FFmpeg decoding solution for the Remote side
> **Applies to**: `mediaservo-codec` (Phase 2+), the `mediaservo-client` decode path
> **References**: BtbN/FFmpeg-Builds (prebuilt), ffmpeg-sys-next (the build.rs pattern), rivet-transcoder (GPU-direct FFI)


---

## 1. Strategy Overview

**Three options in increasing complexity**:

| Option | Complexity | Package size | Flexibility | Suited phase |
|------|--------|--------|--------|---------|
| A: Reference BtbN/FFmpeg-Builds prebuilts | low | ~4 MB | low (fixed codec set) | Phase 1 quick integration |
| B: mediaservo's own prebuilts (the core of this strategy) | medium | ~2 MB | medium (custom codec set) | Phase 2 official plan |
| C: Follow playa-ffmpeg's vcpkg path | low | ~5 MB | low | fallback option |


```
┌─────────────────────────────────────────────────┐
│  Prebuilt layer (CI/Docker)                      │
│  ffmpeg-build/                                   │
│  ├── Dockerfile.macos-arm64  → ffmpeg-arm64.tar  │
│  ├── Dockerfile.macos-x86_64 → ffmpeg-x86_64.tar │
│  ├── Dockerfile.linux-x86_64 → ffmpeg-linux64.tar│
│  ├── Dockerfile.linux-aarch64 → ffmpeg-arm64.tar │
│  └── Dockerfile.windows-msvc → ffmpeg-win64.tar  │
└──────────────────────┬──────────────────────────┘
                       │ upload CI artifacts / S3
┌──────────────────────▼──────────────────────────┐
│  During cargo build                              │
│  build.rs                                        │
│  ├── checks the FFMPEG_DIR env var               │
│  ├── unset → auto-downloads the prebuilt from CI cache │
│  ├── verifies the ABI: pkg-config or header probing │
│  └── emits cargo: directives (rustc-link-search, etc.) │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────┐
│  The Rust binding layer                          │
│  mediaservo-codec::ffmpeg                          │
│  ├── uses the ffmpeg-sys/ffmpeg-next FFI directly │
│  └── implements the VideoDecoder trait           │
└─────────────────────────────────────────────────┘
```

**Core principles**:
- **Never compile FFmpeg sources** — zero wait on cargo build
- **CI prebuilds are the single source of truth** — build once, reused by every developer
- **Fallback: FFMPEG_DIR points at a local build** — developers may compile it themselves
- **Trim to need** — the Remote side needs only decode + colorspace convert

---

## 2. The Prebuilt Scripts (`ffmpeg-build/`)

### 2.1 Directory Structure

```
ffmpeg-build/
├── build.sh               # entry script: detect platform, fetch deps, invoke configure
├── configure-opts.sh      # per-platform configure flags
├── Dockerfile.linux       # Linux x86_64 build container
├── Dockerfile.linux-arm   # Linux aarch64 cross-compile container
├── Dockerfile.macos       # macOS universal binary build
├── Dockerfile.windows     # Windows MSVC cross-compile (via mingw-w64)
├── patch/                 # optional patches
│   └── 0001-static-math.patch
└── README.md
```

### 2.2 Minimal configure flags (per D71 — decode only)

```bash
#!/bin/bash
# configure-opts.sh — the Remote side's minimal decoder set

COMMON_FLAGS=(
    # === disable everything ===
    --disable-everything

    # === enable decoders only ===
    --enable-decoder=h264
    --enable-decoder=hevc
    --enable-decoder=vp8
    --enable-decoder=vp9
    --enable-decoder=opus
    --enable-decoder=aac
    --enable-decoder=mp3
    --enable-decoder=pcm_s16le

    # === enable parsers only ===
    --enable-parser=h264
    --enable-parser=hevc
    --enable-parser=vp8
    --enable-parser=vp9
    --enable-parser=opus
    --enable-parser=aac
    --enable-parser=mpegaudio

    # === enable demuxers only ===
    --enable-demuxer=h264
    --enable-demuxer=hevc
    --enable-demuxer=matroska
    --enable-demuxer=webm_dash_manifest
    --enable-demuxer=ogg
    --enable-demuxer=aac
    --enable-demuxer=mp3
    --enable-demuxer=wav

    # === enable muxers (for testing) ===
    --enable-muxer=null

    # === protocols ===
    --enable-protocol=file
    --enable-protocol=pipe

    # === filters (required for colorspace conversion) ===
    --enable-filter=scale
    --enable-filter=format
    --enable-filter=null

    # === no encoders/hwaccel/postproc ===
    # (the Remote side only decodes; these are unneeded)

    # === binary size optimization ===
    --enable-small
    --disable-doc
    --disable-htmlpages
    --disable-manpages
    --disable-podpages
    --disable-txtpages
    --disable-ffmpeg
    --disable-ffplay
    --disable-ffprobe
    --disable-avdevice
    --disable-postproc
    --disable-swresample        # no audio resampling needed (WebRTC has its own)
    --disable-avfilter          # no filter graphs (colorspace conversion is in swscale)
    --disable-network           # the Remote side never decodes network streams
    --disable-bsfs              # no bitstream filters needed

    # === static libraries ===
    --enable-static
    --disable-shared
    --enable-pic                # position-independent code (needed by Rust static linking)
)

# === platform-specific flags ===
case "$TARGET" in
    *linux*)
        EXTRA_FLAGS=(
            --disable-libxcb
            --disable-libxcb-shm
            --disable-libxcb-xfixes
            --disable-libxcb-shape
            --disable-xlib
            --disable-sdl2
            --disable-alsa
            --disable-pulse
        )
        ;;
    *macos*)
        EXTRA_FLAGS=(
            --disable-securetransport
            --disable-videotoolbox  # ponytail: Apple VT is a dynamic framework; skip in static-linking scenarios
            --disable-avfoundation
            --disable-coreimage
        )
        ;;
    *windows*)
        EXTRA_FLAGS=(
            --disable-mediafoundation
            --disable-schannel
        )
        ;;
esac

ALL_FLAGS=("${COMMON_FLAGS[@]}" "${EXTRA_FLAGS[@]}")
```

### 2.3 The Build Script

```bash
#!/bin/bash
# build.sh — the FFmpeg static-build entry point
set -euo pipefail

FFMPEG_VERSION="${FFMPEG_VERSION:-7.0.2}"
FFMPEG_SRC="https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/out}"
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"

echo "=== Building FFmpeg ${FFMPEG_VERSION} for ${TARGET:-native} ==="

# 1. download the source
if [ ! -d "ffmpeg-${FFMPEG_VERSION}" ]; then
    curl -sSL "$FFMPEG_SRC" | tar xJ
fi

cd "ffmpeg-${FFMPEG_VERSION}"

# 2. Configure
source ../configure-opts.sh
./configure \
    --prefix="$OUTPUT_DIR" \
    "${ALL_FLAGS[@]}"

# 3. Build
make -j"$JOBS"
make install

# 4. Package
cd "$OUTPUT_DIR"
tar czf "ffmpeg-${FFMPEG_VERSION}-${TARGET}.tar.gz" lib/ include/

echo "=== Done: $OUTPUT_DIR/ffmpeg-${FFMPEG_VERSION}-${TARGET}.tar.gz ==="
```

---

## 3. GPL Compliance

### 3.1 Licensing Risk — Key Facts

**libx264 is GPL-licensed, not LGPL!** The moment `./configure` enables `--enable-libx264`, the whole FFmpeg set (libavcodec, libavformat, libavutil, libswscale) flips from LGPL to **GPL 2+**, and the entire linked binary becomes GPL-contaminated.

Likewise libx265 is GPL. ffmpeg-sys-next controls this explicitly via the `build-license-gpl` feature flag, off by default.

| FFmpeg component | Default license | after --enable-libx264 | Impact |
|-------------|---------|---------------------|------|
| libavcodec | LGPL 2.1+ | **GPL 2+** | all binaries GPL |
| libavformat | LGPL 2.1+ | **GPL 2+** | all binaries GPL |
| libavutil | LGPL 2.1+ | **GPL 2+** | all binaries GPL |
| libswscale | LGPL 2.1+ | **GPL 2+** | all binaries GPL |

**Alternative**: use FFmpeg's built-in H.264 decoder (`--enable-decoder=h264`, no libx264 dependency) to stay LGPL. The built-in decoder is patent-encumbered (a separate consideration) but does not trigger GPL at the licensing level.

**Conclusion**: the `configure-opts.sh` above enables no GPL encoders; the pure-decode path stays LGPL.

### 3.2 LGPL Static-Linking Compliance Requirements

Per FSF LGPL 2.1 §6, statically linking an LGPL library requires:

1. **Provide object files**: ship MediaServo's `.o` files so end users can relink against a modified FFmpeg
   - Workable approach: keep the `.rlib` intermediates in CI
2. **Source distribution**: reference the FFmpeg source directly (version + URL); unmodified sources need not be redistributed
3. **Notice**: declare the LGPL components used and how to obtain them in the LICENSE file

### 3.3 Recommended Compliance Setup

```
MediaServo/
├── LICENSE           # Apache 2.0 (application code)
├── LICENSE.3rdparty  # third-party license declarations
│   ├── FFmpeg 7.0.2 — LGPL 2.1+
│   ├── libwebrtc — BSD
│   └── ...
└── NOTICE            # how to obtain the FFmpeg source + how to relink
```

**NOTICE template**:
```
This product includes FFmpeg (version 7.0.2) licensed under LGPL 2.1+.
Source: https://ffmpeg.org/releases/ffmpeg-7.0.2.tar.xz
To relink against a modified FFmpeg, obtain MediaServo object files from:
  <release page URL>
and run: cargo build --target <your-target> -- FFMPEG_DIR=<path>
```

### 3.4 If GPL Encoders Are Ever Needed

If Phase 3+ needs libx264/libx265 encoding on the Remote side:

- **Option A**: dynamically link the GPL libraries (`.so`/`.dylib`) — GPL treats dynamic linking as non-contaminating for the app
- **Option B**: a separate process (an ffmpeg CLI subprocess) — process isolation; GPL doesn't spread
- **Option C**: license everything GPL — not recommended

Current strategy: no decode path depends on GPL; keep the LGPL route.

---

## 4. cargo build Integration

### 4.1 The build.rs Design

`mediaservo-codec/build.rs` (or `mediaservo-client/build.rs`):

```rust
// build.rs — FFmpeg static linking detection
use std::env;
use std::path::PathBuf;

fn main() {
    if !cfg!(feature = "backend-ffmpeg") {
        return;
    }

    let ffmpeg_dir = match env::var("FFMPEG_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            // Try auto-downloading from the CI cache
            let target = env::var("TARGET").unwrap();
            let cache_dir = dirs_next().or_else(|| {
                eprintln!("cargo:warning=FFMPEG_DIR not set and no cache dir found");
                eprintln!("cargo:warning=Set FFMPEG_DIR to point to pre-built FFmpeg");
                eprintln!("cargo:warning=  e.g. FFMPEG_DIR=/opt/ffmpeg cargo build");
                std::process::exit(1);
            });
            // ponytail: CI cache lookup omitted; see ffmpeg-build/ci-download.sh
            eprintln!("cargo:warning=FFMPEG_DIR not set. Pre-built binaries not yet cached.");
            eprintln!("cargo:warning=Download from CI artifacts or build via ffmpeg-build/build.sh");
            std::process::exit(1);
        }
    };

    let lib_dir = ffmpeg_dir.join("lib");
    let include_dir = ffmpeg_dir.join("include");

    // Verify the key files exist
    let required = [
        ("libavcodec", lib_dir.join("libavcodec.a")),
        ("libavformat", lib_dir.join("libavformat.a")),
        ("libavutil", lib_dir.join("libavutil.a")),
        ("libswscale", lib_dir.join("libswscale.a")),
    ];
    for (name, path) in &required {
        if !path.exists() {
            panic!("Missing {name} at {}", path.display());
        }
    }

    // cargo directives
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=avcodec");
    println!("cargo:rustc-link-lib=static=avformat");
    println!("cargo:rustc-link-lib=static=avutil");
    println!("cargo:rustc-link-lib=static=swscale");

    // macOS extra links (VideoToolbox is referenced in headers but unneeded when statically disabled)
    // Linux extra links
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-lib=static=z");
        println!("cargo:rustc-link-lib=static=m");
        println!("cargo:rustc-link-lib=static=pthread");
    }

    // Header path (for bindgen or ffmpeg-sys)
    println!("cargo:include={}", include_dir.display());

    // Compile-time symbol verification (see section 7)
    verify_symbols(&lib_dir);
}

fn verify_symbols(lib_dir: &PathBuf) {
    // Key function-pointer verification — ensures libavcodec has decode capability
    let test_symbols = [
        "avcodec_find_decoder",
        "avcodec_alloc_context3",
        "avcodec_open2",
        "avcodec_send_packet",
        "avcodec_receive_frame",
        "sws_getContext",
        "sws_scale",
    ];
    // ponytail: compile-time symbol verification compiles a small probe via cc;
    // see section 7 for the build.rs probe plan
    let _ = (lib_dir, test_symbols); // placeholder
}
```

### 4.2 Cargo.toml feature flags

```toml
[features]
default = []
# the FFmpeg decode backend (Remote side, statically linked)
backend-ffmpeg = []
# the GStreamer decode backend (general, dynamically linked)
backend-gstreamer = []
```

### 4.3 Code-Level Feature Gates

```rust
// mediaservo-codec/src/lib.rs

/// Unified decoder factory — the backend is chosen at compile time
pub fn create_decoder(config: &DecodeConfig) -> Box<dyn VideoDecoder> {
    #[cfg(feature = "backend-ffmpeg")]
    {
        return Box::new(ffmpeg::FfmpegDecoder::new(config));
    }
    #[cfg(feature = "backend-gstreamer")]
    {
        return Box::new(gstreamer::GstDecoder::new(config));
    }
    #[cfg(not(any(feature = "backend-ffmpeg", feature = "backend-gstreamer")))]
    {
        compile_error!("No decoder backend selected. Enable backend-ffmpeg or backend-gstreamer.");
    }
}
```

---

## 5. CI/CD Strategy

### 5.1 The GitHub Actions Prebuilt Pipeline

```yaml
# .github/workflows/build-ffmpeg.yml
name: Build FFmpeg Static Libraries

on:
  push:
    paths:
      - 'ffmpeg-build/**'
      - '.github/workflows/build-ffmpeg.yml'
  workflow_dispatch:

jobs:
  build:
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            dockerfile: Dockerfile.linux
            artifact: ffmpeg-linux-x86_64.tar.gz
          - target: aarch64-unknown-linux-gnu
            dockerfile: Dockerfile.linux-arm
            artifact: ffmpeg-linux-aarch64.tar.gz
          - target: x86_64-apple-darwin
            dockerfile: Dockerfile.macos
            artifact: ffmpeg-macos-x86_64.tar.gz
            runs-on: macos-13
          - target: aarch64-apple-darwin
            dockerfile: Dockerfile.macos
            artifact: ffmpeg-macos-arm64.tar.gz
            runs-on: macos-14
          - target: x86_64-pc-windows-msvc
            dockerfile: Dockerfile.windows
            artifact: ffmpeg-windows-x86_64.tar.gz

    runs-on: ${{ matrix.runs-on || 'ubuntu-latest' }}
    steps:
      - uses: actions/checkout@v4
      - name: Build FFmpeg
        run: |
          cd ffmpeg-build
          TARGET=${{ matrix.target }} ./build.sh
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: ffmpeg-build/out/*.tar.gz
          retention-days: 90  # ponytail: pre-builds valid for 3 months; long-term goes to S3
```

### 5.2 The Main Build Pipeline (cache reuse)

```yaml
# .github/workflows/ci.yml (snippet)
jobs:
  build:
    steps:
      - uses: actions/checkout@v4
      - name: Download FFmpeg pre-built
        run: |
          # Download the latest artifact from the previous build-ffmpeg workflow run
          # Or download from an S3 bucket (long-term cache)
          mkdir -p /tmp/ffmpeg
          curl -sSL "$FFMPEG_CACHE_URL/$TARGET.tar.gz" | tar xz -C /tmp/ffmpeg
          echo "FFMPEG_DIR=/tmp/ffmpeg" >> $GITHUB_ENV
      - name: cargo build
        run: cargo build --features backend-ffmpeg
```

### 5.3 Developer Local

```bash
# Method 1: download from CI
./ffmpeg-build/ci-download.sh linux-x86_64

# Method 2: build locally (slow, but customizable)
FFMPEG_VERSION=7.0.2 ./ffmpeg-build/build.sh

# Method 3: system package manager (dev only)
brew install ffmpeg  # macOS
apt install libavcodec-dev libavformat-dev libswscale-dev  # Linux
export FFMPEG_DIR=/usr/local  # point at the system install

# then
FFMPEG_DIR=/opt/ffmpeg cargo build --features backend-ffmpeg
```

---

## 6. Cross-Compilation

### 6.1 Strategy

**FFmpeg cross-compilation: on x86_64 CI, use Docker to cross-compile to arm64.**

```dockerfile
# Dockerfile.linux-arm
FROM --platform=linux/arm64 ubuntu:24.04
RUN apt-get update && apt-get install -y \
    build-essential curl xz-utils \
    gcc-aarch64-linux-gnu
# then build FFmpeg with the cross-compiler
```

```bash
# configure-opts.sh — linux-arm64 additions
--cross-prefix=aarch64-linux-gnu-
--target-os=linux
--arch=aarch64
```

### 6.2 Rust Cross-Compile Alignment

```
TARGET_TRIPLE              FFmpeg --target-os  --arch
=========================================================
x86_64-unknown-linux-gnu   linux               x86_64
aarch64-unknown-linux-gnu  linux               aarch64
x86_64-apple-darwin        darwin              x86_64
aarch64-apple-darwin       darwin              aarch64
x86_64-pc-windows-msvc     win32 (mingw-w64)   x86_64
```

**Key**: FFmpeg's cross-compile `--target-os` + `--arch` must match the Rust `TARGET`, otherwise the ABIs are incompatible.

---

## 7. Compile-Time Verification

### 7.1 The build.rs Symbol Probe

```rust
// build.rs (cont.) — compiles a small C program verifying the FFmpeg symbols exist

fn verify_symbols(include_dir: &Path, lib_dir: &Path) {
    use std::process::Command;

    let probe_code = r#"
#include <libavcodec/avcodec.h>
#include <libavutil/frame.h>
int main() {
    // Key: confirm avcodec_find_decoder links
    const AVCodec *c = avcodec_find_decoder(AV_CODEC_ID_H264);
    if (c) {
        // AVFrame fields exist (used by receive_frame)
        AVFrame *f = av_frame_alloc();
        av_frame_free(&f);
        return 0;
    }
    return 1;
}
"#;

    let out_dir = env::var("OUT_DIR").unwrap();
    let probe_c = PathBuf::from(&out_dir).join("ffmpeg_probe.c");
    let probe_exe = PathBuf::from(&out_dir).join("ffmpeg_probe");

    std::fs::write(&probe_c, probe_code).unwrap();

    let status = Command::new(env::var("CC").unwrap_or_else(|_| "cc".into()))
        .arg("-o").arg(&probe_exe)
        .arg(&probe_c)
        .arg(format!("-I{}", include_dir.display()))
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-lavcodec").arg("-lavutil")
        .arg("-lpthread").arg("-lz").arg("-lm")
        .status()
        .expect("failed to run cc for FFmpeg probe");

    if !status.success() {
        panic!(
            "FFmpeg symbol probe failed. Verify FFMPEG_DIR={} has valid static libs.",
            include_dir.parent().unwrap().display()
        );
    }

    // Run the probe (optional; confirms runtime linking)
    let run_status = Command::new(&probe_exe).status()
        .expect("failed to run FFmpeg probe");
    assert!(run_status.success(), "FFmpeg probe returned non-zero");
}
```

### 7.1b ffmpeg-sys-next's `check_features()` pattern (advanced)

```rust
/// Compiles a C probe, parses FFmpeg #define values, drives Rust #[cfg].
/// The core build.rs pattern of ffmpeg-sys-next (reference implementation).
fn check_features(include_dir: &Path) {
    use std::process::Command;

    let probe_code = r"
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>

int main() {
    printf("FFMPEG_VERSION=%s\n", AV_STRINGIFY(LIBAVCODEC_VERSION));
    printf("H264_DECODER=%d\n",
        !!avcodec_find_decoder(AV_CODEC_ID_H264));
    printf("HEVC_DECODER=%d\n",
        !!avcodec_find_decoder(AV_CODEC_ID_HEVC));
    printf("VP9_DECODER=%d\n",
        !!avcodec_find_decoder(AV_CODEC_ID_VP9));
    return 0;
}
"#;

    let out = compile_and_run(probe_code, include_dir);
    // Parse the stdout output and set cargo:rustc-cfg
    for line in out.lines() {
        if line.starts_with("H264_DECODER=1") {
            println!("cargo:rustc-cfg=feature=\"h264_decode\"");
        }
        if line.starts_with("HEVC_DECODER=1") {
            println!("cargo:rustc-cfg=feature=\"hevc_decode\"");
        }
    }
}
```

This lets the Rust side switch decode paths at compile time via `#[cfg(feature = "h264_decode")]`.

### 7.2 CI guard

```yaml
# ci.yml — compile-time verification as a CI step
- name: Verify FFmpeg symbols
  run: |
    FFMPEG_DIR=/tmp/ffmpeg cargo check --features backend-ffmpeg
    # check runs build.rs, triggering the symbol probe
```

---

## 8. Binary Size Optimization

### 8.1 Comparative Estimates

| Scenario | Library size | Notes |
|------|--------|------|
| Full FFmpeg (shared) | ~30 MB | all codecs |
| Full FFmpeg (static) | ~40 MB | static linking grows it |
| Minimal decoder set (static, --enable-small) | ~4 MB | --disable-everything + minimal decoders |
| + LTO + strip | **~2 MB** | Rust target release profile |

### 8.2 recommended release profile

```toml
# Cargo.toml (mediaservo-codec or workspace level)
[profile.release]
opt-level = "z"      # size first (instead of "s")
lto = true            # Rust + FFmpeg cross-library LTO
codegen-units = 1     # maximize the LTO effect
strip = true          # strip symbols
panic = "abort"       # shrink further
```

**Note**: FFmpeg itself needs `--enable-small --disable-debug` build options to benefit.

---

## 9. Reference Project Analysis

### 9.1 ffmpeg-next (zmwangx/rust-ffmpeg)

| Project | Conclusion |
|------|------|
| Linking | pkg-config dynamic linking (`.so`/`.dylib`), not static |
| Applicability | doesn't fit MediaServo Remote's zero-dependency need |
| Borrowable | the feature-flag → codec selection mapping pattern (`ffmpeg_<ver>`) |

### 9.2 GStreamer (the current Host plan)

| Project | Conclusion |
|------|------|
| Current state | the Host has GStreamer integrated, feature-gated |
| Limitations | fully dynamic; the plugin system can't be staticized; unsuitable for Remote |
| Complementarity | Host GStreamer encode + Remote FFmpeg decode — decided in D71 |

### 9.3 rivet-transcoder (reference)

| Project | Conclusion |
|------|------|
| Strategy | prebuild FFmpeg in CI, upload to GitHub Releases, cargo references it via an env var |
| Borrowable | **the artifact-caching pattern**: GitHub Actions → Release → build.rs downloads |
| Build optimization | `--disable-everything` + a minimal codec set; the final .a is ~3-5 MB |

### 9.4 playa-ffmpeg (vcpkg-first)

| Project | Conclusion |
|------|------|
| Strategy | vcpkg auto-installs FFmpeg (Linux/macOS/Windows); choose static linking in the triplet |
| Borrowable | **the zero-config path**: no FFMPEG_DIR needed; vcpkg handles everything |
| Three-platform CI | GitHub Actions proven on Linux + macOS + Windows, with vcpkg cache |
| Build time | first ~15min (vcpkg download+compile of FFmpeg), then ~30s (cache hit) |
| Size | vcpkg's default full build → ~5 MB, trimmable via vcpkg features |
| Applicability | quick prototyping — the fallback when you don't want to manage Docker and FFmpeg builds |

### 9.5 BtbN/FFmpeg-Builds (the prebuilt standard reference)

| Project | Conclusion |
|------|------|
| Strategy | GitHub Actions + Docker + crosstool-ng building **fully static** FFmpeg binaries |
| Coverage | Linux x86_64, Linux arm64, Windows x86_64 (published as prebuilt `.tar.xz`) |
| Borrowable | **the prebuilt-artifact pattern**: build inside Docker → upload to GitHub Releases → build.rs downloads from the URL |
| Strengths | community-maintained, no Dockerfiles to maintain; directly usable as option A |
| Weaknesses | a fixed codec set (full-featured), ~30 MB; no macOS target |

### 9.6 Comparison Matrix

| Dimension | ffmpeg-next (source) | playa-ffmpeg (vcpkg) | BtbN Builds | this strategy (own build) |
|------|---------------------|---------------------|-------------|--------------|
| Build method | git clone + make | vcpkg install | prebuilt download | Docker + make |
| build.rs complexity | 67KB (a full builder) | 5.6KB (vcpkg orchestration) | N/A (must build one) | ~2KB (probe + verify) |
| First-build time | ~10min (source compile) | ~15min (vcpkg compile) | ~30s (download) | ~30s (download prebuilt) |
| Prebuilt size | N/A (self-compiled) | ~5 MB | ~30 MB | ~2-4 MB |
| Cross-compile | first-class (iOS/Android) | vcpkg triplet | Linux/Windows only | Docker cross-compile |
| macOS support | ✅ | ✅ | ❌ (self-build needed) | ✅ (GitHub Actions macos runners) |
| Windows MSVC | ✅ | ✅ (best) | ✅ (mingw-w64) | ✅ |
| Codec customization | ✅ (fully custom) | ✅ (vcpkg features) | ❌ (full-featured) | ✅ (minimal set) |
| GPL control | ✅ (feature flag) | ❌ (the vcpkg port default may include it) | ❌ | ✅ (controlled by configure) |

---

## 10. Implementation Roadmap

| Phase | Task | Output | Effort |
|-------|------|------|--------|
| **0** | FFmpeg prebuilt scripts + Dockerfiles | the `ffmpeg-build/` directory | 2-3 days |
| **0** | CI pipeline: build-ffmpeg.yml | `.github/workflows/` | 1 day |
| **1** | the `mediaservo-codec` crate skeleton | `crates/mediaservo-codec/` | 2 days |
| **1** | build.rs (FFmpeg detection + symbol verification) | `build.rs` | 1 day |
| **1** | the FFmpeg implementation of the VideoDecoder trait | `codec/ffmpeg_decoder.rs` | 3 days |
| **2** | Remote Client decode integration | the `mediaservo-client` decode path | 2 days |
| **2** | binary size optimization + benchmarks | release profile tuning | 1 day |
| **3** | GPL encoder support (if needed) | dynamic linking or subprocess | TBD |

---

## 11. Risks & Mitigations

| Risk | Probability | Mitigation |
|------|------|------|
| FFmpeg ABI instability (major versions) | medium | pin the version (`FFMPEG_VERSION=7.0.2`), CI-controlled |
| Prebuilt packages too large (>10 MB) slowing clones | low | `--enable-small` + a minimal codec set → ~4 MB |
| LGPL static-linking compliance issues | medium | the NOTICE file + keeping the .rlib intermediates |
| macOS signing issues (static libs) | low | `.a` files need no signing; the final binary is signed by Xcode |
| Cross-compile ABI mismatch | medium | Docker-isolated builds, TARGET strictly aligned (see §6.2) |
| System dependencies (libm, libz, libpthread) | low | system libraries present on every target platform |

---

**ponytail summary**:
- Prebuild → build.rs probe → static link → zero runtime dependencies
- Skipped: compiling FFmpeg live, vcpkg (large), full-featured builds
- The Remote side decodes only; no encoder introduced
- GPL boundary: stay LGPL (**libx264 = GPL; never enable it**)
- Quick start: option A (BtbN/FFmpeg-Builds prebuilt) → option B (own build) → option C (vcpkg)

---

## Appendix: Reference Resources

- ffmpeg-sys-next build.rs: https://github.com/russelltg/rust-ffmpeg-sys
- playa-ffmpeg vcpkg integration: https://github.com/ssoj13/playa-ffmpeg
- BtbN/FFmpeg-Builds prebuilts: https://github.com/BtbN/FFmpeg-Builds
- rivet-transcoder GPU architecture: https://github.com/rivet-transcoder/rivet
- FFmpeg official legal: https://www.ffmpeg.org/legal.html
- LGPL compliance checklist: FSF LGPL 2.1 §6a-b (the static-linking clauses)
