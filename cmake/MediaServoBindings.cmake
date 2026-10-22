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

    set(_root "${MEDIASERVO_REPO_ROOT}")
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

    # -- SR4 L1: delivery staging rules for the four SDK cdylibs -------------
    # D241 version triple + N7 strip, absorbed from python (one leg, one
    # owner: the triple loop/copy2/_strip_so/symlink pair in
    # scripts/mediaservo_cli.py are deleted in the same stroke). Versions are
    # passed IN (root project() reads [workspace.package] per C43 -- the
    # supplier never reads Cargo.toml). Chain = cmake's documented
    # install(TARGETS) semantics: real <artifact>.<V> + <artifact>.<M> +
    # dev <artifact>; the cargo-embedded SONAME (libmediaservo_<sdk>.so.0)
    # self-resolves through the .so.0 leg (D241) and the dev name matches the
    # unversioned link the headers/pkg-config advertise.
    # STRIP: --strip alone does NOT reach install(FILES)-shape rows (SR5 P7
    # measurement) -- the kw adds the flag-gated strip legs; the CLI invokes
    # `cmake --install ... --strip`. The dev-tree .so.<MAJOR> symlinks in
    # target/{debug,release} are NOT an install concern (T0 row #2, python
    # stays). No EXPORT on these calls: the delivered Config suite is the
    # hand template (mediaservo::<sdk> INTERFACE targets + declarative
    # COMPONENTS + per-domain SDK_LIST), which the supplier stub+wrapper
    # shape cannot byte-replicate -- SR4 L3 ruling (2b), so only the
    # rendered-file TRANSPORT moved cmake-side.
    # SR4 L2 (T0 #6): each SDK's own C header rides its own delivery call --
    # HEADERS_DIRECTORY copies the CONTENTS of <root> (tree preserved), so
    # bindings/c/mediaservo-<sdk>-c/include/mediaservo/<sdk>.h lands
    # include/mediaservo/<sdk>.h, byte-shape identical to the old python glob
    # copy. DESTINATION "include" is the delivered root (P4 default; spelled
    # for the reader). P5 GLOB_HEADERS stays unused: its rows land under the
    # planner-internal DEST_INCLUDE (include/), unreachable for the
    # include/mediaservo delivery shape -- recorded as a supplier surface note,
    # tree form (P4) is the right tool here anyway.
    foreach(_sdk IN LISTS MEDIASERVO_BINDINGS_SDKS)
        polyorch_rust_install(
            TARGETS mediaservo-${_sdk}-c
            SOVERSION ${PROJECT_VERSION_MAJOR}
            VERSION ${PROJECT_VERSION}
            HEADERS_DIRECTORY "${_root}/bindings/c/mediaservo-${_sdk}-c/include"
            DESTINATION include
            STRIP)
    endforeach()

    # SR4 L2 fan-in (T0 #6 common.h + #7 cxx hpp/recursive tree): six roots
    # stage into the SAME include/mediaservo subtree. P4 is one directory per
    # call and a second polyorch_rust_install per handle would restage the lib
    # rows (two writers -- SR4 1.1 disease), so the fan-in is ONE stock
    # install(DIRECTORY) rule (stock cmake, single owner, zero python):
    #   bindings/c/include                      -> mediaservo/common.h
    #   bindings/cxx/include                    -> mediaservo/{detail,3rdparty}/** (recursive, NOTICE rides)
    #   bindings/cxx/mediaservo-<sdk>-cxx/include -> mediaservo/<sdk>.hpp
    install(DIRECTORY
        "${_root}/bindings/c/include/"
        "${_root}/bindings/cxx/include/"
        "${_root}/bindings/cxx/mediaservo-field-cxx/include/"
        "${_root}/bindings/cxx/mediaservo-link-cxx/include/"
        "${_root}/bindings/cxx/mediaservo-deck-cxx/include/"
        "${_root}/bindings/cxx/mediaservo-client-cxx/include/"
        DESTINATION include)

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

    # SR4 L3 (#9 transport, the pre-ruled row): the rendered manifests reach
    # the delivery tree through install(FILES) rules -- python's copy2 loop is
    # deleted in the same stroke (one leg, one owner). Direction is
    # build-tree -> delivery-tree at INSTALL time (exactly the flow the python
    # copy ran); nothing is copied INTO the build tree, so the F28 rejection
    # stands untouched. Two rules, not one: the .pc and Config pairs have
    # different DESTINATION prefixes.
    install(FILES
        "${_gen}/pkgconfig/mediaservo-field.pc"
        "${_gen}/pkgconfig/mediaservo-link.pc"
        "${_gen}/pkgconfig/mediaservo-deck.pc"
        "${_gen}/pkgconfig/mediaservo-client.pc"
        DESTINATION lib/pkgconfig)
    install(FILES
        "${_gen}/cmake/mediaservoConfig.cmake"
        "${_gen}/cmake/mediaservoConfigVersion.cmake"
        DESTINATION lib/cmake/mediaservo)

    # -- SR4 L3 (T0 #9): transport of the rendered suite becomes install rules --
    # Stock install(FILES) rows (no P-primitive needed; the ORPHAN GUARD re-home
    # lands HERE, not in python). The python copy loop in build bindings died in
    # the same stroke (one leg, one owner). #16's SHAPE itself was already cmake-
    # side since S3 (per-domain renderers above): the D46 P2/P3 EXPORT surface
    # stays unused by this consumer -- swapping the shipped renderers for the
    # supplier's stub/Config generation would be a rewrite for zero delivered
    # gain (ponytail: reuse the code that lives here); e2e-delivery's
    # find_package(COMPONENTS) + version-tuple asserts pin the delivered suite.
    set(_pc_rows "")
    foreach(_sdk IN LISTS MEDIASERVO_BINDINGS_SDKS)
        list(APPEND _pc_rows "${_gen}/pkgconfig/mediaservo-${_sdk}.pc")
    endforeach()
    install(FILES ${_pc_rows} DESTINATION lib/pkgconfig)
    install(FILES
        "${_gen}/cmake/mediaservoConfig.cmake"
        "${_gen}/cmake/mediaservoConfigVersion.cmake"
        DESTINATION lib/cmake/mediaservo)
endfunction()
