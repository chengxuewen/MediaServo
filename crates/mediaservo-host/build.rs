//! Link-arg patch for the vehicle box (see also PIT-199 family).
//!
//! The `--lib` test artifact pulls in libwebrtc.a's remaining objects wholesale
//! (archive-selected on rlib undefined symbols), including the XDG desktop-portal
//! objects that reference `g_object_unref` / glib. Under `-Wl,--as-needed` the
//! glib/gobject dylibs get dropped because those undefined symbols live inside a
//! static archive the linker does not count as "needed" at dylib-scan time —
//! the test link then dies with "DSO missing from command line".
//!
//! Force-loading glib around the link line is neutral for the binaries (glib was
//! already loaded when present) and unblocks the tests once, here, instead of a
//! per-invocation RUSTFLAGS that would rebuild the whole workspace.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Gate to the Jetson-family aarch64 box ONLY, and only when the system glib
    // dev symlink exists: on x86_64 (link-healthy dev boxes, future CI runners)
    // libglib2.0-dev may be absent and -lglib-2.0 would turn into "cannot find".
    let has_system_glib =
        std::path::Path::new("/usr/lib/aarch64-linux-gnu/libglib-2.0.so").exists();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64")
        && has_system_glib
    {
        println!("cargo:rustc-link-arg=-Wl,--no-as-needed");
        // Search the system multiarch dir FIRST: the conda env's gio carries a
        // relative `././libgobject` DT_NEEDED the linker rejects as "DSO missing"
        // (PIT-199 family, conda variant). gio-2.0 itself is needed too — the
        // portal objects reference g_dbus_proxy_call_with_unix_fd_list.
        println!("cargo:rustc-link-arg=-L/usr/lib/aarch64-linux-gnu");
        println!("cargo:rustc-link-arg=-lglib-2.0");
        println!("cargo:rustc-link-arg=-lgobject-2.0");
        println!("cargo:rustc-link-arg=-lgio-2.0");
        println!("cargo:rustc-link-arg=-Wl,--as-needed");
    }
}
