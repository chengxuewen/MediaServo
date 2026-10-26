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
#   * Staging since SR4 L1-L4 + SR6 batch-5: the version triples, strip, headers,
#     .pc/Config transport, the node package, the docs volumes and the stale
#     pip-artifact sweeps are install RULES here. What stays python is pip IMPERATIVE:
#     the wheel BUILD inputs (staging mirror, _libs feeds) and the pip --prefix
#     invocations (ruled-away wheel-content territory, batch-5 census). Renderers
#     consume ${CMAKE_CURRENT_BINARY_DIR}/mediaservo-bindings-manifest/ (F16 lane split).

set(MEDIASERVO_BINDINGS_SDKS field link deck client)
set(MEDIASERVO_BINDINGS_SRC "" CACHE PATH "SR6 batch-5 deploy mirror root (<out>/bindings) — cross-tree deploy passes it; the build assembly leaves it EMPTY (mode reset is explicit at the call site so a stale cache can never flip the mode)")

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
    # SR4 L2+L4 single-call shape: one polyorch_rust_install carries all
    # four delivery handles, and every header root funnels through one
    # fan-in rule. Two supplier semantics force this exact form:
    #   * P6 STALE_GLOB (L4, the 09-17 wheel-sweep family): the keep-manifest
    #     is per-CALL, so the swept destination `lib/` needs a single owner --
    #     four calls each sweeping lib/ would delete the siblings' landed real
    #     files (documented interaction: co-located installs sharing a swept
    #     destination must not rely on each other). TARGETS is multi-valued
    #     (D46): one call lands all twelve names and the sweep runs once,
    #     before the copy rows, deleting version-suffixed corpses that are
    #     not this install's keep set.
    #   * P4 HEADERS_DIRECTORY is single-valued per call; folding the C-shim
    #     roots into the fan-in keeps the one-owner-per-subtree shape and the
    #     byte-identical include/mediaservo layout (contents form: the root's
    #     mediaservo/<sdk>.h lands as include/mediaservo/<sdk>.h).
    # P5 GLOB_HEADERS unused: its rows land under the planner-internal
    # DEST_INCLUDE (include/), not the delivered include/mediaservo shape --
    # recorded as a supplier-surface note.
    polyorch_rust_install(
        TARGETS mediaservo-field-c mediaservo-link-c mediaservo-deck-c mediaservo-client-c
        SOVERSION ${PROJECT_VERSION_MAJOR}
        VERSION ${PROJECT_VERSION}
        STALE_GLOB "libmediaservo_*.so.[0-9]*"
        STRIP)

    # SR4 L2/L4 header fan-in (T0 #6/#7): ten roots stage into the SAME
    # include/mediaservo subtree through ONE stock install(DIRECTORY) rule
    # (stock cmake, single owner, zero python):
    #   bindings/c/include                      -> mediaservo/common.h
    #   bindings/c/mediaservo-<sdk>-c/include   -> mediaservo/<sdk>.h
    #   bindings/cxx/include                    -> mediaservo/{detail,3rdparty}/** (recursive, NOTICE rides)
    #   bindings/cxx/mediaservo-<sdk>-cxx/include -> mediaservo/<sdk>.hpp
    install(DIRECTORY
        "${_root}/bindings/c/include/"
        "${_root}/bindings/c/mediaservo-field-c/include/"
        "${_root}/bindings/c/mediaservo-link-c/include/"
        "${_root}/bindings/c/mediaservo-deck-c/include/"
        "${_root}/bindings/c/mediaservo-client-c/include/"
        "${_root}/bindings/cxx/include/"
        "${_root}/bindings/cxx/mediaservo-field-cxx/include/"
        "${_root}/bindings/cxx/mediaservo-link-cxx/include/"
        "${_root}/bindings/cxx/mediaservo-deck-cxx/include/"
        "${_root}/bindings/cxx/mediaservo-client-cxx/include/"
        DESTINATION include)

    # -- renderers: delivery manifests into the build tree (python stages these) --
    set(_gen "${CMAKE_CURRENT_BINARY_DIR}/mediaservo-bindings-manifest")
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
        message(STATUS "mediaservo-bindings-manifest: mediaservo-${_sdk}.pc @ ${_sdk_ver} (domain crate)")
    endforeach()

    # Config suite version = workspace root project version (documented choice).
    set(VERSION "${PROJECT_VERSION}")
    set(MAJOR "${PROJECT_VERSION_MAJOR}")
    string(REPLACE ";" " " SDK_LIST "${MEDIASERVO_BINDINGS_SDKS}")
    configure_file("${_root}/bindings/c/cmake/mediaservoConfig.cmake.in"
                   "${_gen}/cmake/mediaservoConfig.cmake" @ONLY)
    configure_file("${_root}/bindings/c/cmake/mediaservoConfigVersion.cmake.in"
                   "${_gen}/cmake/mediaservoConfigVersion.cmake" @ONLY)

    # Marker target: `cmake --build --target mediaservo-bindings-manifest` = force re-render;
    # files exist configure-time, the target documents/anchors them.
    add_custom_target(mediaservo-bindings-manifest
        COMMENT "delivery manifests: ${_gen}/pkgconfig + ${_gen}/cmake (rendered at configure)")

    # -- SR4 L3 (T0 #9, the pre-ruled row): transport of the rendered suite becomes
    # install rules. Direction is build-tree -> delivery-tree at INSTALL time (the
    # exact flow python's copy2 ran); nothing lands INTO the build tree, F28 stands.
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

    # -- SR6 batch-5: language-native payload placement, one owner per mode ----------
    # Build assembly (SRC empty): the node package + docs volumes land from the
    # REPO/target artifacts (fresh by construction — the cargo invocations run
    # BEFORE this reconfigure at the call site), plus the stale pip-artifact sweeps
    # the python corpse loops retired into (same order: this --install completes,
    # then pip wheel/pip install re-lays what pip owns). Cross-tree deploy (SRC =
    # <out>/bindings): the mirror legs copy exactly the language-native subtrees of
    # the delivery tree (wheel/, node/, docs/, lib/python3.*/) — the retired deploy
    # sweep loop. A subtree never has rows AND mirror legs registered together
    # (two-writer disease, L3 warning's own words).
    # f8f85db rule: script-mode legs use if(IS_DIRECTORY), never IN_LIST; recursive
    # delete = ${CMAKE_COMMAND} -E rm -rf (file(REMOVE_RECURSIVE) is NOT a file()
    # sub-command — measured on cmake 4.4.4 when the first smoke install died on it).
    if(MEDIASERVO_BINDINGS_SRC)
        foreach(_sub wheel node docs)
            if(IS_DIRECTORY "${MEDIASERVO_BINDINGS_SRC}/${_sub}")
                install(DIRECTORY "${MEDIASERVO_BINDINGS_SRC}/${_sub}" DESTINATION .)
            endif()
        endforeach()
        file(GLOB _pydirs LIST_DIRECTORIES true "${MEDIASERVO_BINDINGS_SRC}/lib/python3.*")
        foreach(_d IN LISTS _pydirs)
            if(IS_DIRECTORY "${_d}")
                install(DIRECTORY "${_d}" DESTINATION lib)
            endif()
        endforeach()
    else()
        # node package (D248 layout node/mediaservo/{mediaservo.node,package.json,
        # lib/index.mjs}). mediaservo.node rides install(FILES), NOT PROGRAMS:
        # --strip is measured NOT to touch FILES-shape rows (SR5 P7) so the retired
        # copy2's exact bytes survive; dlopen needs no exec bit. Missing artifact =
        # skip (python's node_so.exists() polarity; cargo runs _run_or_exit above).
        string(TOLOWER "${CMAKE_BUILD_TYPE}" _prof)
        if(NOT _prof)
            set(_prof debug)
        endif()
        set(_node_art "${_target_dir}/${_prof}/libmediaservo_node.so")
        if(EXISTS "${_node_art}")
            install(FILES "${_node_art}" DESTINATION node/mediaservo RENAME mediaservo.node)
            install(FILES "${_root}/bindings/node/package.json" DESTINATION node/mediaservo)
            install(FILES "${_root}/bindings/node/lib/index.mjs" DESTINATION node/mediaservo/lib)
        else()
            message(STATUS "bindings: target/${_prof}/libmediaservo_node.so absent — node/ package NOT staged this pass")
        endif()
        # docs/sdk-cxx volumes: remove-then-copy per round = deleted-volume residue
        # guard (python's rmtree-then-glob discipline; registration order IS run
        # order — the P1-pack measured fact).
        install(CODE "execute_process(COMMAND \"\${CMAKE_COMMAND}\" -E rm -rf \"\${CMAKE_INSTALL_PREFIX}/docs/sdk-cxx\")")
        install(DIRECTORY "${_root}/docs/reference/sdk-cxx" DESTINATION docs)
        # stale pip-artifact sweeps (09-17 e2e-delivery catch: unordered glob +
        # cross-run residue = the rename chain eats its own source; V1b + 10-22
        # corpse catches: pip --prefix never deletes old data_files). pip re-lays
        # the fresh wheel/site-packages immediately after this pass (python side).
        install(CODE "
            file(GLOB _whls \"\${CMAKE_INSTALL_PREFIX}/wheel/*.whl\")
            foreach(_w IN LISTS _whls)
                file(REMOVE \"\${_w}\")
                message(STATUS \"  stale wheel swept\")
            endforeach()
            file(GLOB _spdirs LIST_DIRECTORIES true \"\${CMAKE_INSTALL_PREFIX}/lib/python3.*/site-packages/mediaservo\")
            file(GLOB _dists LIST_DIRECTORIES true \"\${CMAKE_INSTALL_PREFIX}/lib/python3.*/site-packages/mediaservo-*.dist-info\")
            foreach(_d IN LISTS _spdirs _dists)
                if(IS_DIRECTORY \"\${_d}\")
                    execute_process(COMMAND \"\${CMAKE_COMMAND}\" -E rm -rf \"\${_d}\")
                    message(STATUS \"  stale site-packages tree swept\")
                endif()
            endforeach()
        ")
    endif()
endfunction()
