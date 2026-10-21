# MediaServoBindings.cmake — bindings lanes + delivery manifest renderers (S3)
#
# Scope (parent-repo docs/plans/polyorch-adoption/PLAN.md §4 S3, as-built):
#   * Lanes: the four C-shim cdylibs + the napi cdylib ride the supplier SHARED
#     kind as IDE/CI build buttons. BASE_DIR is the deliberate F17-D exception:
#     the workspace target/ — python staging, the build-c task, check-abi-drift,
#     test-cxx and e2e-delivery all consume exactly those paths; a separate
#     lane cache would mint a second copy of every .so instead of sharing one.
#     Lane argv = the task's argv (same package/profile/feature resolution),
#     so no fingerprint thrash (S0(c) parity channel).
#   * Renderers (the handoff from python): per-sdk pkg-config files stamped
#     with the DOMAIN crate version (polyorch_rust_package_version resolves
#     workspace inheritance via cargo metadata — F12/V1a domains decouple on
#     the next independent bump), plus the mediaservo Config/ConfigVersion pair.
#     Config carries the SUITE version (= root project() = workspace.package):
#     one file cannot stamp four domains — recorded as-built deviation.
#   * NOT wired: polyorch_rust_cbindgen as a verify gate — the repo has no
#     cbindgen baseline (headers are hand-frozen; README's intent line was
#     never pipelined). The drift gate is check-abi-drift.sh (header decls vs
#     cdylib dynsyms), which stays THE comparison.
#   * Staging (version triples, strip, wheel, node pack, docs copy) stays
#     python — incident-forged; it consumes ${CMAKE_CURRENT_BINARY_DIR}/
#     bindings-gen/ instead of rendering templates inline (F16 lane split).

set(MEDIASERVO_BINDINGS_SDKS field link deck client)

function(mediaservo_setup_bindings)
    if(NOT MEDIASERVO_BUILD_BINDINGS)
        return()
    endif()
    if(NOT POLYORCH_RUST_FOUND)
        polyorch_rust_setup()   # system route: cargo resolved from the caller env
    endif()

    set(_root "${CMAKE_SOURCE_DIR}")
    set(_target_dir "${_root}/target")   # shared cache — see header for the why

    # -- lanes: 4 C shims + node napi (SHARED kind; build buttons + artifact handles) --
    foreach(_sdk IN LISTS MEDIASERVO_BINDINGS_SDKS)
        polyorch_rust_build(
            TARGET mediaservo-${_sdk}-c SHARED
            PACKAGE mediaservo-${_sdk}-c
            CRATE mediaservo_${_sdk}
            MANIFEST "${_root}/bindings/c/mediaservo-${_sdk}-c/Cargo.toml"
            BASE_DIR "${_target_dir}"
            FOLDER bindings)
    endforeach()
    polyorch_rust_build(
        TARGET mediaservo-node SHARED
        PACKAGE mediaservo-node
        CRATE mediaservo_node
        MANIFEST "${_root}/bindings/node/rust/mediaservo-node/Cargo.toml"
        BASE_DIR "${_target_dir}"
        FOLDER bindings)

    # -- renderers: delivery manifests into the build tree (python stages these) --
    set(_gen "${CMAKE_CURRENT_BINARY_DIR}/bindings-gen")
    file(MAKE_DIRECTORY "${_gen}/pkgconfig" "${_gen}/cmake")

    foreach(_sdk IN LISTS MEDIASERVO_BINDINGS_SDKS)
        polyorch_rust_package_version(
            PACKAGE mediaservo-${_sdk}
            MANIFEST "${_root}/crates/mediaservo-${_sdk}/Cargo.toml"
            OUT_VAR _sdk_ver)
        set(VERSION "${_sdk_ver}")
        configure_file(
            "${_root}/bindings/c/mediaservo-${_sdk}-c/mediaservo-${_sdk}.pc.in"
            "${_gen}/pkgconfig/mediaservo-${_sdk}.pc" @ONLY)
        message(STATUS "bindings-gen: mediaservo-${_sdk}.pc @ ${_sdk_ver} (domain crate)")
    endforeach()

    # Config suite version = workspace root project version (documented choice).
    set(VERSION "${PROJECT_VERSION}")
    set(MAJOR "${PROJECT_VERSION_MAJOR}")
    string(REPLACE ";" " " SDK_LIST "${MEDIASERVO_BINDINGS_SDKS}")
    configure_file("${_root}/bindings/c/cmake/mediaservoConfig.cmake.in"
                   "${_gen}/cmake/mediaservoConfig.cmake" @ONLY)
    configure_file("${_root}/bindings/c/cmake/mediaservoConfigVersion.cmake.in"
                   "${_gen}/cmake/mediaservoConfigVersion.cmake" @ONLY)

    # Marker target: `cmake --build --target bindings-gen` = force re-render;
    # files exist configure-time, the target documents/anchors them.
    add_custom_target(bindings-gen
        COMMENT "delivery manifests: ${_gen}/pkgconfig + ${_gen}/cmake (rendered at configure)")
endfunction()
