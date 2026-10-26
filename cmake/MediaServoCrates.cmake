# MediaServoCrates.cmake — crates lanes: IDE buttons + ctest bridge (S4)
#
# Contract (parent-repo docs/plans/polyorch-adoption/PLAN.md §4 S4):
#   * THE lane table (single source) — `<crate>@<lib-name>@<folder>` rows over the
#     workspace's crates/* members. The mediaservo-gates lanes face
#     (tools/mediaservo-gates, SR6 P3) reconciles this table
#     against the root Cargo.toml members every all-gates run (drift = FATAL),
#     so the table can never rot silently.
#   * cargo keeps build authority: this file registers driver surface only
#     (LIB-kind rlib lanes + test buttons + ctest twins) — zero compilation rules,
#     the lanes shell `cargo` through the supplier command wrapper.
#   * cache posture (F17-D, narrowed by SR2/D44): default = supplier build-tree-local
#     `.cargo-target` + honest whole-face clean buttons. Under
#     `-DMEDIASERVO_SHARE_CARGO_TARGET=ON` lanes point at the workspace target/
#     (shared with bare cargo, S0(c) steady alternation); a WHOLE-FACE clean button
#     is never registered there (rm -rf would annihilate the authoritative gate
#     cache) — per-package `cargo clean -p` buttons through the supplier wrapper
#     (D44) are the sanctioned shared-mode form.
#   * posture rule (F18): default features only — no stub variant lanes; the
#     gate entry (bare-cargo all-gates) is untouched (§3/§7.4 invariant).
#   * aarch64 (C23/S0b): the command wrapper strips host CC/CXX, so lanes here
#     inject the system toolchain explicitly when a Jetson-class host carries it;
#     without /usr/bin/gcc the lane face skips itself LOUDLY (documented posture,
#     never silently broken).
#   * ctest bridge: the verb's own CTEST keyword (supplier D45) registers the
#     entry from the SAME command vector as the button — the former
#     sanctioned-internal exception (_polyorch_rust_command reads; ModuKit W4
#     precedent) is RETIRED here: this file touches no supplier internals.
#   * `polyorch_rust_run` is deliberately NOT wired for host/server packages:
#     instance lifecycle belongs to C39 (deployed binaries / systemctl), not to
#     IDE run buttons.

# App-binary lanes (S6): the three CLI-consumed executables. BINARY handles over
# the workspace target/ — deliberate F17-D exception like bindings: build:deploy,
# systemd units, and doctor all read target/<profile>/<bin>; a separate face would
# mint a second copy nobody deploys. No ctest twin (package -test compiles bins too);
# `${_app}-bin-clean` exists in shared mode only — an alias of the crate-row -p
# command (N6 button symmetry with -bin-build; corr runs lib+bin buttons off ONE
# package-scoped command: cargo has no bins-only selector, -p covers rlib+exe
# together). The separate face keeps its own whole-dir wipe instead.
# app-binary lanes = crates that actually ship a bin named like the package
# (cargo metadata 2026-10-26: host/server yes; client is lib-only since the
# S6 SDK split moved its binary face to the cdylib delivery -- keeping a
# client row here produced a permanently red button, first click 10-26).
set(MEDIASERVO_APP_BINS mediaservo-host mediaservo-server)

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
        set(_base "${MEDIASERVO_REPO_ROOT}/target")
            else()
        set(_base "${CMAKE_BINARY_DIR}/.cargo-target")  # supplier default face, spelled
    endif()

    foreach(_row IN LISTS MEDIASERVO_CRATE_LANES)
        string(REPLACE "@" ";" _parts "${_row}")
        list(GET _parts 0 _pkg)
        list(GET _parts 1 _lib)
        list(GET _parts 2 _fold)
        set(_manifest "${MEDIASERVO_REPO_ROOT}/${_fold}/Cargo.toml")

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
            FOLDER "${_fold}"
            # D45 ctest entry: one vector with the button (default name is
            # exactly ${_pkg}-test -- the 11-name census must stay identical).
            # WORKING_DIRECTORY pins the workspace root (entry semantics held
            # since S4, NOT the D45 default manifest-parent); LABELS keep -L/-R
            # routing quiet for the gate suites; SKIP 77 = OUR PIT-192 posture.
            CTEST CTEST_LABELS "rust;lanes"
            CTEST_WORKING_DIRECTORY "${MEDIASERVO_REPO_ROOT}"
            CTEST_SKIP_RETURN_CODE 77)

        if(MEDIASERVO_SHARE_CARGO_TARGET)
            # Shared mode (F17-D/SR2): per-package `cargo clean -p` through the
            # sanctioned wrapper (supplier D44) — manifest-parent cwd, sibling
            # artifacts provably untouched (t-rust-clean-e2e judge).
            polyorch_rust_clean(NAME ${_pkg}-clean PACKAGE ${_pkg}
                MANIFEST "${_manifest}" ${_base_args} FOLDER "${_fold}")
        else()
            # Separate face: whole-face honest clean (all lanes share one
            # build-tree-local dir) — wiping it IS the clean, zero-toolchain form.
            polyorch_rust_clean(NAME ${_pkg}-clean ${_base_args} FOLDER "${_fold}")
        endif()

    endforeach()

    # App-binary lanes (see the table note above).
    foreach(_app IN LISTS MEDIASERVO_APP_BINS)
        set(_found OFF)
        foreach(_row IN LISTS MEDIASERVO_CRATE_LANES)
            string(REPLACE "@" ";" _p "${_row}")
            list(GET _p 0 _c)
            if(_c STREQUAL _app)
                set(_found ON)
                break()
            endif()
        endforeach()
        if(NOT _found)
            message(FATAL_ERROR
                "MEDIASERVO_APP_BINS entry '${_app}' is not in MEDIASERVO_CRATE_LANES — "
                "the two tables must move together (add the rlib lane first).")
        endif()
        polyorch_rust_build(
            TARGET ${_app}-bin BINARY
            PACKAGE ${_app} CRATE ${_app}
            MANIFEST "${MEDIASERVO_REPO_ROOT}/crates/${_app}/Cargo.toml"
            BASE_DIR "${MEDIASERVO_REPO_ROOT}/target"
            FOLDER "crates/${_app}")
        if(_c23_inject)
            polyorch_rust_set_env_vars(TARGET ${_app}-bin ${_c23_inject})
        endif()
        if(MEDIASERVO_SHARE_CARGO_TARGET)
            # N6 alias pair with -bin-build (user ruling 2026-10-22): deliberately
            # the SAME package-scoped command as ${_app}-clean above — registering
            # the -p clean twice is idempotent by construction. Shared-branch gate
            # REQUIRED: BASE_DIR hard-codes the workspace target/ here while the
            # loop runs on BOTH faces.
            polyorch_rust_clean(NAME ${_app}-bin-clean PACKAGE ${_app}
                MANIFEST "${MEDIASERVO_REPO_ROOT}/crates/${_app}/Cargo.toml"
                BASE_DIR "${MEDIASERVO_REPO_ROOT}/target"
                FOLDER "crates/${_app}")
        endif()
    endforeach()

    if(MEDIASERVO_SHARE_CARGO_TARGET)
        message(STATUS
            "MediaServo crates lanes: shared workspace target/ "
            "(clean = per-package cargo clean -p, D44)")
    else()
        message(STATUS
            "MediaServo crates lanes: separate build-tree face "
            "(clean = whole-face rm -rf)")
    endif()
endfunction()
