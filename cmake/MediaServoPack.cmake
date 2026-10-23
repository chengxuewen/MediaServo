# MediaServoPack.cmake — SR6 P1: the release tars described natively (CPack TGZ only)
#
# Contract: parent-repo docs/plans/polyorch-sr6-python-retirement/PLAN.md §P1 (a)-(e).
#   * Gate (cache var, SR7/SR4 pattern): MEDIASERVO_PKG "" | host | server | sdk-field |
#     sdk-client. Empty registers ZERO rules — every preset/IDE/gates configure stays
#     behavior-neutral. Legs migrate one at a time (one-leg-one-owner): an unmigrated
#     value FATALs instead of silently shipping a hollow tar.
#   * Naming owner = HERE (CPACK_PACKAGE_FILE_NAME doubles as the tar top-level dir):
#     {brand}-{pkg}-{ver}-{platform}-{arch} — the C42 + 2026-10-19 platform-suffix
#     formula byte-ported from python (_TARGET_CRATE version sources, x86_64->amd64
#     map, sys.platform slot). The CLI reads package-root.txt back (single source;
#     python computes no second copy of the name).
#   * Staging owner = `cmake --install --prefix <staging>` — install(SCRIPT/CODE) legs
#     then see CMAKE_INSTALL_PREFIX as the live staging root (the design (e) premise;
#     no CPack project-mode DESTDIR indirection). Packing owner = `cpack --config`
#     with CPACK_INSTALLED_DIRECTORIES over that staged tree (measured on this cmake:
#     the archive wrapper dir is CPACK_PACKAGE_FILE_NAME itself, symlinks and empty
#     directories survive).
#   * Install-time legs are configured templates under cmake/pack-legs/*.cmake.in —
#     configure_file @ONLY embeds constants; the staged-tree live state (readelf,
#     domain sweeps) is read at install time. install events run in registration
#     order (measured): placement rows first, state/manifest legs after.
#   * f8f85db rule: leg scripts stay script-mode policy-neutral (list(FIND), never
#     IN_LIST) — same discipline as MediaServoAppTree.cmake.

set(MEDIASERVO_PKG "" CACHE STRING "SR6-P1 release package to describe at install time: '' | host | server | sdk-field | sdk-client")
set(MEDIASERVO_PKG_BRAND "" CACHE STRING "brand slot for the tar name (the CLI's --brand; empty = mediaservo)")
set(MEDIASERVO_PKG_ALIAS "" CACHE STRING "naming-slot override for the one-cycle bindings->sdk alias")
set(MEDIASERVO_BIND_OUT "" CACHE PATH "SDK delivery tree (<out>/bindings) — the sdk-* package sources")
set(MEDIASERVO_OXMGR_BIN "" CACHE PATH "CLI-resolved oxmgr binary (host/server packages — vendor copy row)")

# Install-time leg templates live next to this file. CMAKE_CURRENT_LIST_DIR inside a
# function is the CALLER's listfile (root CMakeLists), so anchor it here at include
# processing time instead.
set(MEDIASERVO_PACK_LEGS "${CMAKE_CURRENT_LIST_DIR}/pack-legs")

# Crate version reader (python _crate_version port): the quoted [package] version;
# workspace-inherited or absent = root project version (C43 single source).
function(_msr_crate_version crate out_var)
    file(READ "${MEDIASERVO_REPO_ROOT}/crates/${crate}/Cargo.toml" _t)
    string(REGEX MATCH "\\[package\\][^\\[]*" _seg "${_t}")
    string(REGEX MATCH "(^|\n)[ \t]*version[ \t]*=[ \t]*\"([^\"]+)\"" _m "${_seg}")
    if(_m)
        set(${out_var} "${CMAKE_MATCH_2}" PARENT_SCOPE)
    else()
        set(${out_var} "${PROJECT_VERSION}" PARENT_SCOPE)
    endif()
endfunction()

# Integer constant from Rust source (python shape-drift abort parity: missing = FATAL).
function(_msr_src_int file regex out_var label)
    file(READ "${file}" _t)
    string(REGEX MATCH "${regex}" _m "${_t}")
    if(NOT _m)
        message(FATAL_ERROR "Error: ${label} not found in ${file} (shape drift)")
    endif()
    set(${out_var} "${CMAKE_MATCH_1}" PARENT_SCOPE)
endfunction()

function(mediaservo_setup_pack)
    if(MEDIASERVO_PKG STREQUAL "sdk-client")
        _msr_pack_sdk("sdk-client" "mediaservo-client" "client")
    elseif(MEDIASERVO_PKG STREQUAL "sdk-field")
        _msr_pack_sdk("sdk-field" "mediaservo-field" "field;link;deck")
    elseif(MEDIASERVO_PKG STREQUAL "host")
        _msr_pack_host()
    elseif(MEDIASERVO_PKG STREQUAL "server")
        _msr_pack_server()
    else()
        message(FATAL_ERROR
            "MEDIASERVO_PKG='${MEDIASERVO_PKG}' is not one of host|server|sdk-field|sdk-client")
    endif()
endfunction()

# -- shared contract-file legs (version.txt + CHANGES.md; targets differ only in
#    the naming slot and the changelog keep set) --
function(_msr_pack_contract_files domain ver pkg_name keep)
    _msr_src_int("${MEDIASERVO_REPO_ROOT}/crates/mediaservo-link/src/frame.rs"
                 "pub const WIRE_VERSION: u8 = ([0-9]+)" _fw "WIRE_VERSION constant")
    _msr_src_int("${MEDIASERVO_REPO_ROOT}/crates/mediaservo-link/src/token.rs"
                 "pub const VERSION: u8 = ([0-9]+)" _tv "TokenFile::VERSION")
    _msr_src_int("${MEDIASERVO_REPO_ROOT}/crates/mediaservo-common/src/protocol.rs"
                 "pub const SIGNALING_PROTOCOL_VERSION: u32 = ([0-9]+)" _spv "SIGNALING_PROTOCOL_VERSION")
    _msr_src_int("${MEDIASERVO_REPO_ROOT}/crates/mediaservo-common/src/protocol.rs"
                 "pub const SIGNALING_PROTOCOL_MIN_SUPPORTED: u32 = ([0-9]+)" _smin "SIGNALING_PROTOCOL_MIN_SUPPORTED")

    set(_gen "${CMAKE_CURRENT_BINARY_DIR}/pkg-gen")
    set(PKG_NAME "${pkg_name}")
    set(VER "${ver}")
    set(FW "${_fw}")
    set(TV "${_tv}")
    set(SPV "${_spv}")
    set(SMIN "${_smin}")
    configure_file("${MEDIASERVO_PACK_LEGS}/version-txt.cmake.in"
                   "${_gen}/version-txt.cmake" @ONLY)
    install(SCRIPT "${_gen}/version-txt.cmake")

    set(CHANGELOG "${MEDIASERVO_REPO_ROOT}/CHANGELOG.md")
    set(KEEP "${keep}")
    configure_file("${MEDIASERVO_PACK_LEGS}/changes.cmake.in"
                   "${_gen}/changes.cmake" @ONLY)
    install(SCRIPT "${_gen}/changes.cmake")

    if(domain)
        set(DOMAIN "${domain}")
        configure_file("${MEDIASERVO_PACK_LEGS}/manifest.cmake.in"
                       "${_gen}/manifest.cmake" @ONLY)
        install(SCRIPT "${_gen}/manifest.cmake")
    endif()
endfunction()

function(_msr_pack_naming crate pkg_default out_ver out_rootname out_pkgname)
    _msr_crate_version("${crate}" _ver)
    set(_pkg_name "${pkg_default}")
    if(MEDIASERVO_PKG_ALIAS)
        set(_pkg_name "${MEDIASERVO_PKG_ALIAS}")
    endif()
    set(_nb "mediaservo")
    if(MEDIASERVO_PKG_BRAND)
        set(_nb "${MEDIASERVO_PKG_BRAND}")
    endif()
    if(WIN32)
        set(_plat "win32")
    elseif(APPLE)
        set(_plat "darwin")
    else()
        set(_plat "linux")
    endif()
    string(TOLOWER "${CMAKE_SYSTEM_PROCESSOR}" _mach)
    if(_mach STREQUAL "x86_64" OR _mach STREQUAL "amd64")
        set(_arch "amd64")
    elseif(_mach STREQUAL "aarch64" OR _mach STREQUAL "arm64")
        set(_arch "arm64")
    else()
        set(_arch "${_mach}")
    endif()
    set(${out_ver} "${_ver}" PARENT_SCOPE)
    set(${out_pkgname} "${_pkg_name}" PARENT_SCOPE)
    set(_rootname "${_nb}-${_pkg_name}-${_ver}-${_plat}-${_arch}")
    set(${out_rootname} "${_rootname}" PARENT_SCOPE)
    # the CLI reads this back for the staging dir + expected tar (naming owner = here)
    file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/package-root.txt" "${_rootname}")
endfunction()

# -- sdk halves (V1b dual-package; source = the single out/bindings delivery tree) --
function(_msr_pack_sdk domain crate sdks)
    _msr_pack_naming("${crate}" "${domain}" _ver _rootname _pkg_name)

    # precondition parity with _stage_sdk_package (python error text kept verbatim)
    if(NOT MEDIASERVO_BIND_OUT OR NOT EXISTS "${MEDIASERVO_BIND_OUT}/lib")
        message(FATAL_ERROR "Error: out/bindings does not exist — run build bindings first")
    endif()

    # lib: the real versioned file is a row; the dev/major symlinks ride the legs
    # with the SOURCE tree's exact link targets (no SOVERSION semantics invented).
    set(_specs "")
    foreach(_sdk IN LISTS sdks)
        file(GLOB _cands "${MEDIASERVO_BIND_OUT}/lib/libmediaservo_${_sdk}.so*")
        set(_real "")
        foreach(_f IN LISTS _cands)
            if(IS_SYMLINK "${_f}")
                file(READ_SYMLINK "${_f}" _tgt)
                get_filename_component(_bn "${_f}" NAME)
                list(APPEND _specs "${_bn}=${_tgt}")
            elseif(NOT _real)
                set(_real "${_f}")
            endif()
        endforeach()
        if(NOT _real)
            message(FATAL_ERROR
                "Error: ${MEDIASERVO_BIND_OUT}/lib/libmediaservo_${_sdk}.so.<ver> missing — run build bindings first")
        endif()
        install(FILES "${_real}" DESTINATION lib)
    endforeach()

    # shared payload: whole-copy rows + the domain sweep leg (python's copytree+prune
    # net state; these subtrees carry no symlinks)
    install(DIRECTORY "${MEDIASERVO_BIND_OUT}/include/mediaservo" DESTINATION include)
    install(DIRECTORY "${MEDIASERVO_BIND_OUT}/lib/pkgconfig" DESTINATION lib)
    install(DIRECTORY "${MEDIASERVO_BIND_OUT}/docs" DESTINATION .)

    # per-domain Config pair (python template .replace port; @ONLY expands the
    # in-comment occurrences exactly like python did)
    set(VERSION "${_ver}")
    string(REGEX MATCH "^([0-9]+)" _maj "${_ver}")
    set(MAJOR "${_maj}")
    string(REPLACE ";" " " SDK_LIST "${sdks}")
    set(_gen "${CMAKE_CURRENT_BINARY_DIR}/pkg-gen")
    configure_file("${MEDIASERVO_REPO_ROOT}/bindings/c/cmake/mediaservoConfig.cmake.in"
                   "${_gen}/mediaservoConfig.cmake" @ONLY)
    configure_file("${MEDIASERVO_REPO_ROOT}/bindings/c/cmake/mediaservoConfigVersion.cmake.in"
                   "${_gen}/mediaservoConfigVersion.cmake" @ONLY)
    install(FILES "${_gen}/mediaservoConfig.cmake" "${_gen}/mediaservoConfigVersion.cmake"
            DESTINATION lib/cmake/mediaservo)

    # language-native payloads (design (c)): pip/npm still BUILD them; sdk-field absorbs
    # the staged dirs, sdk-client ships none
    if(domain STREQUAL "sdk-field")
        file(GLOB _pydirs LIST_DIRECTORIES true "${MEDIASERVO_BIND_OUT}/lib/python3.*")
        foreach(_d IN LISTS _pydirs)
            if(IS_DIRECTORY "${_d}")
                install(DIRECTORY "${_d}" DESTINATION lib)
            endif()
        endforeach()
        install(DIRECTORY "${MEDIASERVO_BIND_OUT}/wheel" DESTINATION .)
        install(DIRECTORY "${MEDIASERVO_BIND_OUT}/node" DESTINATION .)
    endif()

    # legs: links+sweep first (manifest then reads the landed dev .so names), then the
    # contract files (version.txt, CHANGES.md, manifest.json)
    set(DOMAIN "${domain}")
    set(LINK_SPECS "${_specs}")
    configure_file("${MEDIASERVO_PACK_LEGS}/sdk-legs.cmake.in"
                   "${_gen}/sdk-legs.cmake" @ONLY)
    install(SCRIPT "${_gen}/sdk-legs.cmake")

    if(domain STREQUAL "sdk-client")
        set(_keep "sdk-client protocol")
    else()
        set(_keep "sdk-field protocol")
    endif()
    _msr_pack_contract_files("${domain}" "${_ver}" "${_pkg_name}" "${_keep}")
endfunction()

# -- server tree (L3): SR7 app rows (the CLI passes -DMEDIASERVO_APP_TREE=server)
#    plus stateful legs — vendor copy row, web move, lifecycle probe, init render,
#    strip (design (e): placement owner = AppTree rules, state owner = the staged
#    binary; python's _cmd_deploy_server staging sequence ported 1:1 in order).
function(_msr_pack_server)
    if(NOT MEDIASERVO_APP_TREE STREQUAL "server")
        message(FATAL_ERROR
            "MEDIASERVO_PKG=server requires -DMEDIASERVO_APP_TREE=server (the CLI package verb passes both)")
    endif()
    if(NOT MEDIASERVO_OXMGR_BIN)
        message(FATAL_ERROR
            "MEDIASERVO_PKG=server requires -DMEDIASERVO_OXMGR_BIN (CLI _resolve_oxmgr_or_die owns the hunt)")
    endif()
    _msr_pack_naming("mediaservo-server" "server" _ver _rootname _pkg_name)
    set(_exe "${CMAKE_EXECUTABLE_SUFFIX}")
    if(MEDIASERVO_BRAND)
        set(_sname "${MEDIASERVO_BRAND}-server${_exe}")
    else()
        set(_sname "mediaservo-server${_exe}")
    endif()

    # oxmgr vendor copy (python copied it after the app --install; registration
    # order here reproduces that — the AppTree bin-sweep above never targets oxmgr)
    install(PROGRAMS "${MEDIASERVO_OXMGR_BIN}" DESTINATION bin RENAME "oxmgr${_exe}")

    # the web tree moves wholesale (caddy static root; copytree parity)
    install(DIRECTORY "${MEDIASERVO_SERVER_OUT_DIR}/web/" DESTINATION web
            USE_SOURCE_PERMISSIONS)

    set(_gen "${CMAKE_CURRENT_BINARY_DIR}/pkg-gen")
    set(SRV_NAME "${_sname}")
    configure_file("${MEDIASERVO_PACK_LEGS}/server-legs.cmake.in"
                   "${_gen}/server-legs.cmake" @ONLY)
    install(SCRIPT "${_gen}/server-legs.cmake")

    _msr_pack_contract_files("" "${_ver}" "${_pkg_name}" "server protocol deploy")
endfunction()

# -- host tree (L4): SR7 app rows (8 branded bins + reclaim + entry link) plus
#    init (identity/tokens — state owner = the staged binary), the generated env.sh,
#    run/logs + recordings dirs and the PIT-119 strip.
function(_msr_pack_host)
    if(NOT MEDIASERVO_APP_TREE STREQUAL "host")
        message(FATAL_ERROR
            "MEDIASERVO_PKG=host requires -DMEDIASERVO_APP_TREE=host (the CLI package verb passes both)")
    endif()
    if(NOT MEDIASERVO_OXMGR_BIN)
        message(FATAL_ERROR
            "MEDIASERVO_PKG=host requires -DMEDIASERVO_OXMGR_BIN (CLI _resolve_oxmgr_or_die owns the hunt)")
    endif()
    _msr_pack_naming("mediaservo-host" "host" _ver _rootname _pkg_name)
    set(_exe "${CMAKE_EXECUTABLE_SUFFIX}")
    if(MEDIASERVO_BRAND)
        set(_hostdn "${MEDIASERVO_BRAND}-host${_exe}")
        set(_rust_brand "${MEDIASERVO_BRAND}")
    else()
        set(_hostdn "mediaservo-host${_exe}")
        set(_rust_brand "mediaservo")
    endif()

    install(PROGRAMS "${MEDIASERVO_OXMGR_BIN}" DESTINATION bin RENAME "oxmgr${_exe}")

    set(_gen "${CMAKE_CURRENT_BINARY_DIR}/pkg-gen")
    set(HOST_BIN_NAME "${_hostdn}")
    set(RUST_BRAND "${_rust_brand}")
    configure_file("${MEDIASERVO_PACK_LEGS}/host-legs.cmake.in"
                   "${_gen}/host-legs.cmake" @ONLY)
    install(SCRIPT "${_gen}/host-legs.cmake")

    _msr_pack_contract_files("" "${_ver}" "${_pkg_name}" "host protocol deploy")
endfunction()
