# MediaServoCrates.cmake — crates lanes: IDE buttons + ctest bridge (S4)
#
# Contract (parent-repo docs/plans/polyorch-adoption/PLAN.md §4 S4):
#   * THE lane table (single source) — `<crate>@<lib-name>@<folder>` rows over the
#     workspace's crates/* members. scripts/gate-lanes.py reconciles this table
#     against the root Cargo.toml members every all-gates run (drift = FATAL),
#     so the table can never rot silently.
#   * cargo keeps build authority: this file registers driver surface only
#     (LIB-kind rlib lanes + test buttons + ctest twins) — zero compilation rules,
#     the lanes shell `cargo` through the supplier command wrapper.
#   * cache posture (F17-D): default = supplier build-tree-local `.cargo-target`
#     + honest whole-face clean buttons. Under `-DMEDIASERVO_SHARE_CARGO_TARGET=ON`
#     lanes point at the workspace target/ (shared with bare cargo, S0(c) steady
#     alternation) and clean buttons are NOT registered — rm -rf there would
#     annihilate the authoritative gate cache (RustHelpers:2042+ wipes the whole
#     dir, no supplier guard).
#   * posture rule (F18): default features only — no stub variant lanes; the
#     gate entry (bare-cargo all-gates) is untouched (§3/§7.4 invariant).
#   * aarch64 (C23/S0b): the command wrapper strips host CC/CXX, so lanes here
#     inject the system toolchain explicitly when a Jetson-class host carries it;
#     without /usr/bin/gcc the lane face skips itself LOUDLY (documented posture,
#     never silently broken).
#   * ctest bridge: reuses `_polyorch_rust_command` (pixi PATH prefix + env
#     strip) — the same internal the button uses — so button and ctest entry can
#     never drift in env handling (ModuKit W4 precedent: the ONLY sanctioned
#     internal-surface read here; duplicating the strip recipe is forbidden).
#   * `polyorch_rust_run` is deliberately NOT wired for host/server packages:
#     instance lifecycle belongs to C39 (deployed binaries / systemctl), not to
#     IDE run buttons.

set(MEDIASERVO_CRATE_LANES
    "mediaservo-common@mediaservo_common@crates/mediaservo-common"
    "mediaservo-webrtc@mediaservo_webrtc@crates/mediaservo-webrtc"
    "mediaservo-codec@mediaservo_codec@crates/mediaservo-codec"
    "mediaservo-media@mediaservo_media@crates/mediaservo-media"
    "mediaservo-link@mediaservo_link@crates/mediaservo-link"
    "mediaservo-deck@mediaservo_deck@crates/mediaservo-deck"
    "mediaservo-field@mediaservo_field@crates/mediaservo-field"
    "mediaservo-host@mediaservo_host@crates/mediaservo-host"
    "mediaservo-client@mediaservo_client@crates/mediaservo-client"
    "mediaservo-server@mediaservo_server@crates/mediaservo-server"
    "mediaservo-weaknet@mediaservo_weaknet@crates/mediaservo-weaknet")

function(mediaservo_setup_crate_lanes)
    if(NOT MEDIASERVO_BUILD_CRATES_LANES)
        return()
    endif()
    if(NOT POLYORCH_RUST_FOUND)
        polyorch_rust_setup()
    endif()

    # C23 posture on aarch64 (see header): inject, or skip loudly.
    set(_c23_inject "")
    if(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64)$")
        if(EXISTS "/usr/bin/gcc")
            set(_c23_inject CC=/usr/bin/gcc CXX=/usr/bin/g++)
        else()
            message(STATUS
                "MediaServo crates lanes: aarch64 host without /usr/bin/gcc (C23) — "
                "lane face SKIPPED (host cargo builds stay authoritative; install the "
                "system toolchain or keep lanes off on this box)")
            return()
        endif()
    endif()

    # F17-D cache posture (see header): both forms spell the dir explicitly so that
    # build/test/clean/ctest agree on ONE path.
    if(MEDIASERVO_SHARE_CARGO_TARGET)
        set(_base "${CMAKE_SOURCE_DIR}/target")
        set(_clean OFF)   # clean button would rm -rf the gate cache (see header)
    else()
        set(_base "${CMAKE_BINARY_DIR}/.cargo-target")  # supplier default face, spelled
        set(_clean ON)
    endif()

    foreach(_row IN LISTS MEDIASERVO_CRATE_LANES)
        string(REPLACE "@" ";" _parts "${_row}")
        list(GET _parts 0 _pkg)
        list(GET _parts 1 _lib)
        list(GET _parts 2 _fold)
        set(_manifest "${CMAKE_SOURCE_DIR}/${_fold}/Cargo.toml")

        set(_base_args "")
        if(_base)
            set(_base_args BASE_DIR "${_base}")
        endif()

        # LIB kind (R3/D42): cargo --lib rlib lane; INTERFACE handle carries
        # ordering + the artifact path, never staged/installed/run.
        polyorch_rust_build(
            TARGET ${_pkg} LIB
            PACKAGE ${_pkg} CRATE ${_lib}
            MANIFEST "${_manifest}" ${_base_args}
            FOLDER "${_fold}")
        if(_c23_inject)
            polyorch_rust_set_env_vars(TARGET ${_pkg} ${_c23_inject})
        endif()

        polyorch_rust_test(
            PACKAGE ${_pkg} NAME ${_pkg}-test TARGET ${_pkg}
            MANIFEST "${_manifest}" ${_base_args}
            FOLDER "${_fold}")

        if(_clean)
            # Whole-face honest clean (all lanes share one face dir) — same
            # semantics the siblings document: never a sibling-only wipe.
            polyorch_rust_clean(NAME ${_pkg}-clean ${_base_args} FOLDER "${_fold}")
        endif()

        # ctest twin: SAME argv + SAME command wrapper as the button (ModuKit W4
        # shape). Labels keep -L/-R routing quiet for the gate suites.
        _polyorch_rust_forward_env(TRIPLE "${POLYORCH_RUST_HOST_TARGET}" OUT_ENV _fenv)
        _polyorch_rust_command(_cmd SUBCOMMAND
            test --package "${_pkg}" --manifest-path "${_manifest}"
            --target-dir "${_base}" ENV "${_fenv}")
        add_test(NAME ${_pkg}-test COMMAND ${_cmd})
        set_tests_properties(${_pkg}-test PROPERTIES
            WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
            SKIP_RETURN_CODE 77
            LABELS "rust;lanes")
    endforeach()

    if(MEDIASERVO_SHARE_CARGO_TARGET)
        set(_mode "shared workspace target/")
    else()
        set(_mode "separate build-tree face")
    endif()
    message(STATUS "MediaServo crates lanes: ${_mode} (clean buttons ${_clean})")
endfunction()
