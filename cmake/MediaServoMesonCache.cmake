# MediaServoMesonCache.cmake -- pinned meson wrap layer discovery (Plan-Y family).
#
# mediasoup-sys builds the C++ worker through meson; its subproject wraps
# (libuv / openssl / libsrtp3 / abseil-cpp / flatbuffers / unordered-dense)
# fetch sources AND wrapdb patches at build time. The wrapdb patch URLs
# 301-redirect to GitHub releases (measured: an IDE deploy-server build
# without the CLI proxy env died on the redirect's TLS handshake -- the IDE
# face never rides msrtc.sh). The parent shell pins the full archive set in
# 3rdparty/mediasoup-worker/ (per-file .sha256 ledger; the hashes are the
# crate's own .wrap pins), and meson consumes it through
# MESON_PACKAGE_CACHE_DIR: sources AND patches resolve from the cache dir by
# filename (wrap.py _get_file_internal), so a complete layer means zero
# network.
#
# PURGE HAZARD (measured): after a successful worker build, mediasoup-sys
# build.rs runs `invoke clean-all` -> `meson subprojects purge
# --include-cache`, which DELETES the archives inside MESON_PACKAGE_CACHE_DIR
# -- pointing it straight at the pinned layer ate the layer on first build.
# The upstream-sanctioned escape is MEDIASOUP_LOCAL_DEV (non-empty skips the
# clean-all leg; also keeps the extracted subprojects for fast incremental
# rebuilds). The env list below therefore ALWAYS pairs the cache dir with
# MEDIASOUP_LOCAL_DEV=1.
#
# Structural discovery, IDE/CLI parity (same posture as the gui-deps sibling
# layer): the archive dir is a SIBLING of this repo in the parent checkout
# (<parent>/3rdparty/{MediaServo,mediasoup-worker}). No layer (standalone
# MediaServo) -> empty list -> the upstream download behavior stays in
# charge. An explicit MESON_PACKAGE_CACHE_DIR in the environment always wins
# (and still gets the LOCAL_DEV pairing -- the purge hazard is identical).

# mediaservo_meson_cache_env(<out>) -- KEY=VALUE entries to inject into cargo
# rules, or an empty list when no pinned layer exists.
function(mediaservo_meson_cache_env out)
    set(_msmc_dir "")
    if(DEFINED ENV{MESON_PACKAGE_CACHE_DIR} AND NOT "$ENV{MESON_PACKAGE_CACHE_DIR}" STREQUAL "")
        set(_msmc_dir "$ENV{MESON_PACKAGE_CACHE_DIR}")
    else()
        get_filename_component(_msmc_layer "${MEDIASERVO_REPO_ROOT}/../../3rdparty/mediasoup-worker" ABSOLUTE)
        if(IS_DIRECTORY "${_msmc_layer}")
            file(GLOB _msmc_archives "${_msmc_layer}/*.zip" "${_msmc_layer}/*.tar.gz")
            if(_msmc_archives)
                set(_msmc_dir "${_msmc_layer}")
            endif()
        endif()
    endif()
    if(_msmc_dir)
        set(${out} "MESON_PACKAGE_CACHE_DIR=${_msmc_dir}" "MEDIASOUP_LOCAL_DEV=1" PARENT_SCOPE)
    else()
        set(${out} "" PARENT_SCOPE)
    endif()
endfunction()
