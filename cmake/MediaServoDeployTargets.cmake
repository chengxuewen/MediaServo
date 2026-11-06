# MediaServoDeployTargets.cmake — deploy buttons (build-env-vsc-affinity, K1b)
#
# Three IDE/CLI-visible targets that drive the _msvc_app_install choreography
# through DEDICATED binaryDirs (never the lane tree — SR7: no lane tree can
# re-lay an app tree; `cmake --build --target` never runs install rules, so the
# target body drives configure+build+install itself):
#
#   mediaservo-deploy-host      crates-lanes gated
#   mediaservo-deploy-server    crates-lanes gated (+ web via APP_STAGING=build)
#   mediaservo-deploy-bindings  bindings gated; PURE install (no init legs)
#   (mediaservo-deploy-client does not exist by design — client is lib-only,
#    delivery = sdk-client package, F12.)
#
# Cache-var contract (resolved HERE at main-tree configure time; targets take
# no argv):
#   MEDIASERVO_DEPLOY_PREFIX_HOST/SERVER  default ${repo}/out/<t>, env
#                                         MSRTC_OUT_ROOT fallback (seat-2 F2)
#   MEDIASERVO_DEPLOY_PROFILE             Debug|Release (U6 channel)
#   MEDIASERVO_BRAND                      -D > env fallback > "" ; `mediaservo`
#                                         maps to "" (legacy host-* layout,
#                                         bash twin mediaservo.sh:1297-1314)
#   MEDIASERVO_OXMGR_BIN                  find_program PATH -> ~/.cache/oxmgr
#                                         -> FATAL (pack-leg contract, Media-
#                                         ServoPack.cmake:236/:273)
#   MEDIASERVO_SERVER_OUT_DIR             default <prefix>/server (AppTree:66-72)
#   MEDIASERVO_DEPLOY_GUARD_MODE          refuse|teardown (R10-K-a: condition 1
#                                         stops the live instance before
#                                         install; conditions 2/3 still
#                                         refuse (ops-F1 stays blocked)
#
#
# DESTINATION & BRAND RULES (R2, user-facing — mirrored in the deployment doc):
#   mediaservo-deploy-* (submodule view)  -> <submodule>/out/<t>, upstream names
#   msrtc-deploy-* (parent view, R5)      -> <parent>/out/<t>, msrtc names
#   msrtc.sh / mediaservo.sh deploy       -> shell env injection decides
#   judges/tests                          -> explicit /tmp prefixes ONLY
# R3: libraries NEVER carry the brand (libmediaservo_*.so stands; brand =
# deployment face = binary/unit names only — D269/C47).
# R4 brand stickiness: a manual -DMEDIASERVO_BRAND in a deploy binaryDir pins
# over later env injection (cache persists across re-configures); the guards
# fail closed on brand conflicts; remedy = delete build/deploy-*.
# Parent-context semantics (C1): mediaservo-* buttons keep upstream semantics
# in EVERY view — the name is the promise. msrtc-deploy-bindings is a symmetry
# button (prefix-only, no brand). "bindings" and "bindings/c|node" render as
# SIBLING tree nodes (shared prefix, not parent/child). Since plan
# ide-folder-hierarchy P6 the web lanes join the mediaservo/web family (setup-
# function knob + explicit aggregate-button FOLDER) — no longer root-level.
#
# Fail-closed guard — THREE conditions, FATAL before install (comment anchor:
# bash twin mediaservo.sh:1317-1324; ops-F1 brandless-reclaim):
#   (1) live instance: <p>/run/oxfile.toml AND <p>/bin/<branded-cli>
#   (2) brand EMPTY while <p>/bin holds *-agent branded binaries
#   (3) prefix root sanity (non-empty, not the repo root)
# Guidance names the bash escape hatch.

# ---- resolved inputs (main-tree configure time) ----

if(DEFINED ENV{MSRTC_OUT_ROOT} AND NOT MEDIASERVO_DEPLOY_PREFIX_HOST)
    set(MEDIASERVO_DEPLOY_PREFIX_HOST "$ENV{MSRTC_OUT_ROOT}/host" CACHE PATH
        "deploy target prefix (env fallback)" FORCE)
endif()
if(NOT MEDIASERVO_DEPLOY_PREFIX_HOST)
    set(MEDIASERVO_DEPLOY_PREFIX_HOST "${MEDIASERVO_REPO_ROOT}/out/host" CACHE PATH
        "deploy target prefix" FORCE)
endif()
if(DEFINED ENV{MSRTC_OUT_ROOT} AND NOT MEDIASERVO_DEPLOY_PREFIX_SERVER)
    set(MEDIASERVO_DEPLOY_PREFIX_SERVER "$ENV{MSRTC_OUT_ROOT}/server" CACHE PATH
        "deploy target prefix (env fallback)" FORCE)
endif()
if(NOT MEDIASERVO_DEPLOY_PREFIX_SERVER)
    set(MEDIASERVO_DEPLOY_PREFIX_SERVER "${MEDIASERVO_REPO_ROOT}/out/server" CACHE PATH
        "deploy target prefix" FORCE)
endif()

set(MEDIASERVO_DEPLOY_PROFILE "Debug" CACHE STRING
    "deploy build profile: Debug | Release (U6)")
set_property(CACHE MEDIASERVO_DEPLOY_PROFILE PROPERTY
    STRINGS Debug Release)

# guard mode: refuse (default) = three-condition FATAL, byte-identical to the
# pre-K-a guard; teardown (R10-K-a) = condition 1 stops the live instance
# (unit -> daemon -> family, CWD-scoped) before install; conditions 2/3 still
# refuse in BOTH modes (ops-F1 resurrection blocked). Cache var, not baked
# into the rendered script = the kill-switch needs no code revert (§10).
set(MEDIASERVO_DEPLOY_GUARD_MODE "refuse" CACHE STRING
    "deploy guard mode: refuse | teardown (R10-K-a)")
set_property(CACHE MEDIASERVO_DEPLOY_GUARD_MODE PROPERTY
    STRINGS refuse teardown)

if(NOT MEDIASERVO_BRAND AND DEFINED ENV{MEDIASERVO_BRAND})
    set(MEDIASERVO_BRAND "$ENV{MEDIASERVO_BRAND}" CACHE STRING
        "deploy brand (env fallback)" FORCE)
endif()
# bash parity: explicit `mediaservo` maps to the legacy upstream-name layout
if(MEDIASERVO_BRAND STREQUAL "mediaservo")
    set(MEDIASERVO_BRAND "" CACHE STRING "deploy brand (mediaservo->legacy empty)" FORCE)
endif()

if(NOT MEDIASERVO_OXMGR_BIN)
    find_program(MEDIASERVO_OXMGR_BIN oxmgr)
endif()
if(NOT MEDIASERVO_OXMGR_BIN)
    file(GLOB _msdt_oxmgr_candidates
         "$ENV{HOME}/.local/bin/oxmgr"
         "$ENV{HOME}/.cache/oxmgr/*/oxmgr")
    list(LENGTH _msdt_oxmgr_candidates _n)
    if(_n GREATER 0)
        list(GET _msdt_oxmgr_candidates 0 MEDIASERVO_OXMGR_BIN)
    endif()
    unset(_msdt_oxmgr_candidates)
endif()
if(NOT MEDIASERVO_OXMGR_BIN)
    message(FATAL_ERROR
        "mediaservo-deploy-*: oxmgr binary not found (PATH, ~/.local/bin, ~/.cache/oxmgr/*) — "
        "the oxmgr version-locked copy leg needs it (MediaServoPack.cmake:236/:273 contract); "
        "or pass -DMEDIASERVO_OXMGR_BIN=<path>")
endif()

if(NOT MEDIASERVO_SERVER_OUT_DIR)
    set(MEDIASERVO_SERVER_OUT_DIR "${MEDIASERVO_DEPLOY_PREFIX_SERVER}/server" CACHE PATH
        "server delivery root for the server target" FORCE)
endif()

# F1 (audit): capture the template dir AT INCLUDE TIME — CMAKE_CURRENT_LIST_DIR
# inside a function is the CALLER's dir, so parent-invoked callsites (R5) would
# FATAL every configure_file below. All template paths ride this variable.
set(_MSDT_TEMPLATE_DIR "${CMAKE_CURRENT_LIST_DIR}" CACHE INTERNAL
    "deploy template dir (F1: captured at include time; CACHE INTERNAL = visible
     at ANY callsite scope — a plain set() is function-dynamic-scoped and the
     parent-brand callsites resolve it to empty)")

set(MEDIASERVO_DEPLOY_STRIP "" CACHE STRING
    "bindings deploy --strip flavor passthrough (empty = none; bindings face has no strip mechanism today — declared K1b finding, not invented)")

function(_mediaservo_deploy_guard_script _out _prefix _brand)
    set(GUARD_MODE "${MEDIASERVO_DEPLOY_GUARD_MODE}")
    set(GUARD_PREFIX "${_prefix}")
    set(GUARD_BRAND "${_brand}")
    if(_brand)
        set(GUARD_IS_BRANDED 1)
    else()
        set(GUARD_IS_BRANDED 0)
    endif()
    set(GUARD_PROFILE "${MEDIASERVO_DEPLOY_PROFILE}")
    configure_file("${_MSDT_TEMPLATE_DIR}/deploy-guard.cmake.in"
                   "${_out}" @ONLY)
endfunction()

# otherwise make parent callsites register nothing under the default preset).

# bindings: AppTree has no bindings tree — the bindings face IS an install face
# (MediaServoBindings rows in the bindings-lane tree). Dedicated binaryDir with
# the bindings gate ON; pure install, no init legs, no guard-needed state (no
# run/oxfile in a bindings prefix; strip = no mechanism today, declared).
# R6 NOTE: this face drives its own shape — configure(bindings gate) -> build
# manifest -> install. The shared function below carries a bindings branch for
# the parent-brand buttons (same three-step shape, parameterized); its AppTree
# choreography (lanes+guard+init) stays host/server-only.
if(MEDIASERVO_BUILD_BINDINGS)
    set(_bdir_b "${MEDIASERVO_REPO_ROOT}/build/deploy-bindings")
    set(_cfg_b "${MEDIASERVO_DEPLOY_PROFILE}")
    if(_cfg_b STREQUAL "Release")
        set(_cfg_b Release)
    else()
        set(_cfg_b Debug)
    endif()
    add_custom_target(mediaservo-deploy-bindings
        COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir_b}
            -DMEDIASERVO_BUILD_BINDINGS=ON -DCMAKE_BUILD_TYPE=${_cfg_b}
            -DMEDIASERVO_APP_TREE=
        COMMAND ${CMAKE_COMMAND} --build ${_bdir_b}
            --target mediaservo-bindings-manifest
        COMMAND ${CMAKE_COMMAND} --install ${_bdir_b} --prefix ${MEDIASERVO_DEPLOY_PREFIX_HOST}
        COMMENT "mediaservo deploy bindings -> ${MEDIASERVO_DEPLOY_PREFIX_HOST} (bindings manifest + delivery rows)"
        VERBATIM)
    set_target_properties(mediaservo-deploy-bindings PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}deploy")
endif()

# ---- target registration (R6: one parameterized function, audit signature) ----
# mediaservo_add_deploy_target(TREE <host|server|bindings>
#                              PREFIX <path>      REQUIRED — no cache fallback:
#                                                 the caller pins the destination
#                                                 (submodule callsites pass the
#                                                 resolved cache vars; parent
#                                                 callsites pin their own out)
#                              BRAND <string>     REQUIRED — empty = upstream
#                                                 names (explicit "" allowed)
#                              FOLDER <dir>       optional IDE folder; empty =
#                                                 unset (lets the parent view's
#                                                 CMAKE_FOLDER carry the target;
#                                                 audit ide-semantics: explicit
#                                                 FOLDER wins wholesale)
#                              SERVER_OUT_DIR <p> optional (server only; derived
#                                                 from PREFIX when tree=server)
#                              STAGING <s>        optional MEDIASERVO_APP_STAGING
#                              INIT <script>      optional post-install init leg
#                                                 (a cmake -P script path; host/
#                                                 server render theirs internally
#                                                 from the pack-legs templates)
# GATE is NOT part of the public form — module callsites decide registration by
# calling this function only when their lane flag is ON (audit: the gate would
# otherwise make parent callsites register nothing under the default preset).
function(_mediaservo_add_deploy_target)
    cmake_parse_arguments(MDT "" "TREE;NAME;PREFIX;BRAND;FOLDER;SERVER_OUT_DIR;STAGING;INIT;BUILD_ROOT" "" ${ARGN})
    # cmake_parse_arguments UNDEFINES a keyword given an empty-string value
    # (BRAND "" = the upstream-name form, a legal input) — so DEFINED cannot
    # distinguish "not passed" from "passed empty". Require TREE + PREFIX only;
    # BRAND defaults to "" (upstream names) when absent.
    if(NOT MDT_TREE OR NOT DEFINED MDT_PREFIX)
        message(FATAL_ERROR "_mediaservo_add_deploy_target: TREE and PREFIX are required")
    endif()
    if(NOT DEFINED MDT_BRAND)
        set(MDT_BRAND "")
    endif()
    # build-root: deploy scratch binaryDir anchored per-context at the callsite
    # (parent -> ms_rtc/build, submodule -> MediaServo/build). Default = submodule build
    # for any caller that omits it; deterministic (no ${CMAKE_BINARY_DIR}/../ drift).
    if(NOT MDT_BUILD_ROOT)
        set(MDT_BUILD_ROOT "${MEDIASERVO_REPO_ROOT}/build")
    endif()
    set(_t "${MDT_TREE}")
    # target name: mediaservo-deploy-<tree> by default; the parent brand
    # buttons pass NAME msrtc-deploy-<tree> (CMP0002 — the submodule
    # registration already owns the mediaservo-* names)
    set(_name "mediaservo-deploy-${_t}")
    if(MDT_NAME)
        set(_name "${MDT_NAME}")
    endif()
    set(_pfx "${MDT_PREFIX}")
    set(_brand "${MDT_BRAND}")
    if(_brand STREQUAL "mediaservo")
        set(_brand "")
    endif()
    set(_bdir "${MDT_BUILD_ROOT}/deploy-${_t}")
    set(_profile "${MEDIASERVO_DEPLOY_PROFILE}")
    if(_profile STREQUAL "Release")
        set(_cfg Release)
    else()
        set(_cfg Debug)
    endif()
    # bindings rides its OWN shape here (R6 NOTE above): AppTree has no
    # bindings tree — the face is configure(bindings gate) -> build manifest
    # -> install, mirroring the submodule's own mediaservo-deploy-bindings
    # one-to-one (name/prefix/build-root parameterized). No goods leg (the
    # manifest build produces its artifacts through the bindings face, whose
    # cargo rides PolyOrch's built-in PIT-14 bridge), no guard, no init legs.
    # The shared AppTree choreography below stays host/server-only —
    # -DMEDIASERVO_APP_TREE=bindings would FATAL the AppTree gate
    # ('' |host|server), which is how the parent button died on first click
    # from the day it was registered (found 2026-11-05).
    if(_t STREQUAL "bindings")
        add_custom_target(${_name}
            COMMAND ${CMAKE_COMMAND} -E echo
                "deploy-bindings: configure dedicated binaryDir ${_bdir} (profile ${_profile})"
            COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir}
                -DMEDIASERVO_BUILD_BINDINGS=ON -DCMAKE_BUILD_TYPE=${_cfg}
                -DMEDIASERVO_APP_TREE=
            COMMAND ${CMAKE_COMMAND} --build ${_bdir}
                --target mediaservo-bindings-manifest
            COMMAND ${CMAKE_COMMAND} --install ${_bdir} --prefix ${_pfx}
            COMMENT "mediaservo deploy bindings -> ${_pfx} (bindings manifest + delivery rows)"
            VERBATIM)
        if(MDT_FOLDER)
            set_target_properties(${_name} PROPERTIES FOLDER "${MDT_FOLDER}")
        endif()
        return()
    endif()
    set(_extra_defs "")
    set(_extra_commands "")
    if(NOT _t STREQUAL "bindings")
        _mediaservo_deploy_guard_script(
            "${CMAKE_BINARY_DIR}/deploy-${_name}-guard.cmake" "${_pfx}" "${_brand}")
        # R10-K-a: the host face runs the guard in TEARDOWN mode (stop-first
    # inside the cmake choreography — IDE button == bash leg behavior);
    # server/bindings keep the caller-selected mode (refuse default).
    if(_t STREQUAL "host")
        set(_guard_mode "teardown")
    else()
        set(_guard_mode "${MEDIASERVO_DEPLOY_GUARD_MODE}")
    endif()
    set(_guard_args
            COMMAND ${CMAKE_COMMAND}
                -DMEDIASERVO_DEPLOY_GUARD=1
                -DMEDIASERVO_DEPLOY_GUARD_PREFIX=${_pfx}
                -DMEDIASERVO_DEPLOY_GUARD_BRAND=${_brand}
                -DMEDIASERVO_DEPLOY_GUARD_MODE=${_guard_mode}
                -P ${CMAKE_BINARY_DIR}/deploy-${_name}-guard.cmake)
    else()
        set(_guard_args "")
    endif()
    if(_t STREQUAL "host")
        if(_brand)
            set(HOST_BIN_NAME "${_brand}-host")
        else()
            set(HOST_BIN_NAME "mediaservo-host")
        endif()
        set(RUST_BRAND "${_brand}")
        set(OXMGR_BIN "${MEDIASERVO_OXMGR_BIN}")   # D-H13 version-locked copy leg (host-legs.cmake.in)
        configure_file("${_MSDT_TEMPLATE_DIR}/pack-legs/host-legs.cmake.in"
                       "${CMAKE_BINARY_DIR}/deploy-${_name}-init.cmake" @ONLY)
        file(TO_CMAKE_PATH "${_pfx}" _pfx_cm)
        file(READ "${CMAKE_BINARY_DIR}/deploy-${_name}-init.cmake" _init)
        string(PREPEND _init "set(CMAKE_INSTALL_PREFIX \"${_pfx_cm}\")\n")
        file(WRITE "${CMAKE_BINARY_DIR}/deploy-${_name}-init.cmake" "${_init}")
        set(_init_args
            COMMAND ${CMAKE_COMMAND}
                -DRUN_ROOT=${_pfx}
                -DHOST_BIN_NAME=${HOST_BIN_NAME}
                -DRUST_BRAND=${_brand}
                -P ${CMAKE_BINARY_DIR}/deploy-${_name}-init.cmake)
    elseif(_t STREQUAL "server")
        if(MDT_SERVER_OUT_DIR)
            set(_srv_out "${MDT_SERVER_OUT_DIR}")
        else()
            set(_srv_out "${_pfx}")   # audit F5: derived from the callsite prefix
        endif()
        if(_brand)
            set(SRV_NAME "${_brand}-server")
        else()
            set(SRV_NAME "mediaservo-server")
        endif()
        set(RUST_BRAND "${_brand}")
        configure_file("${_MSDT_TEMPLATE_DIR}/pack-legs/server-legs.cmake.in"
                       "${CMAKE_BINARY_DIR}/deploy-${_name}-init-server-body.cmake" @ONLY)
        file(WRITE "${CMAKE_BINARY_DIR}/deploy-${_name}-init.cmake"
             "set(CMAKE_INSTALL_PREFIX \"${_pfx}\")
include(\"${CMAKE_BINARY_DIR}/deploy-${_name}-init-server-body.cmake\")
")
        set(_extra_defs
            -DMEDIASERVO_SERVER_OUT_DIR=${_srv_out})
        if(MDT_STAGING)
            set(_extra_defs ${_extra_defs} -DMEDIASERVO_APP_STAGING=${MDT_STAGING})
            # build-assembly staging skips the root entry links by contract;
            # this face is a DEPLOY (assembly + landing in one button), so it
            # opts in — the IDE button lands <prefix>/{brand}-server like the
            # CLI deploy landing does (msrtc-deploy-server parity gap,
            # 2026-11-05).
            set(_extra_defs ${_extra_defs} -DMEDIASERVO_APP_ENTRY_LINKS=ON)
        endif()
        set(_init_args
            COMMAND ${CMAKE_COMMAND}
                -P ${CMAKE_BINARY_DIR}/deploy-${_name}-init.cmake)
    else()
        set(_init_args "")
    endif()
    if(MDT_INIT)
        # caller-provided init leg (unused today; parent faces may pin one)
        set(_init_args ${_init_args} COMMAND ${CMAKE_COMMAND} -P "${MDT_INIT}")
    endif()
    # Fresh-goods leg (2026-11-05, C+ ruling, inline form): the deploy
    # sequence OPENS with the CLI `build <tree>` cargo invocation — the FULL
    # bin set into the SHARED workspace target/ (hard-coded on purpose, same
    # posture as the app-bin lanes: the calling tree's isolated .cargo-target
    # base never leaks into deploy goods). Inline COMMAND, not a separate
    # goods target: the IDE target tree stays as flat as the CLI verb list
    # (one deploy button per tree, zero helper targets), and cargo still runs
    # incrementally before the pure-install pass — never ships stale bins.
    # This also breaks the first-run deadlock the old shape had: the dedicated
    # tree used to carry its own lanes face, and AppTree's configure-time bin
    # hunt fired on a .cargo-target/ that only the (never reached) build step
    # would have populated. (A per-bin lane handle would NOT do here:
    # mediaservo-host is a multi-bin crate and the BINARY lanes build one
    # named bin each — 1 of 8.)
    set(_goods_pkgs "")
    if(_t STREQUAL "host")
        set(_goods_pkgs mediaservo-host mediaservo-client)   # CLI _ms_build_host parity
    elseif(_t STREQUAL "server")
        set(_goods_pkgs mediaservo-server)
    endif()
    set(_goods_args "")
    if(_goods_pkgs)
        if(NOT POLYORCH_RUST_CARGO)
            message(FATAL_ERROR "deploy ${_name}: fresh-goods leg needs "
                "POLYORCH_RUST_CARGO (the crates lanes face resolves it — "
                "deploy targets register only under that gate)")
        endif()
        # Env bridge mirrors the rust-build face's own invocation shape
        # (RustHelpers PIT-14 block + PATH prepend + libclang pin): cargo can
        # spawn rustc, bindings build scripts find libclang, and a host-
        # activated shell cannot leak compiler shaping into the goods build.
        # MESON/MESON_ARGS too: conda gcc activation exports
        # MESON_ARGS=-Dbuildtype=release, which collides with mediasoup-sys's
        # own --buildtype and panics the worker build (pixi.toml
        # build-server-native precedent; button envs are naturally clean —
        # only the CLI shell rides the pixi activation, B5 measured).
        get_filename_component(_cargo_dir "${POLYORCH_RUST_CARGO}" DIRECTORY)
        set(_goods_cmd
            ${CMAKE_COMMAND} -E env
            --unset=RUSTFLAGS --unset=CARGO_ENCODED_RUSTFLAGS
            --unset=CFLAGS --unset=CXXFLAGS --unset=CC --unset=CXX
            --unset=MESON --unset=MESON_ARGS
            "PATH=${_cargo_dir}:/usr/bin:/bin"
            "LIBCLANG_PATH=${_cargo_dir}/../lib/libclang.so"
            "${POLYORCH_RUST_CARGO}" build)
        foreach(_p IN LISTS _goods_pkgs)
            list(APPEND _goods_cmd -p "${_p}")
        endforeach()
        list(APPEND _goods_cmd
            --manifest-path "${MEDIASERVO_REPO_ROOT}/Cargo.toml"
            --target-dir "${MEDIASERVO_REPO_ROOT}/target")
        if(_cfg STREQUAL "Release")
            list(APPEND _goods_cmd --release)
        endif()
        set(_goods_args
            COMMAND ${CMAKE_COMMAND} -E echo
                "deploy-${_t}: fresh goods — cargo build [${_goods_pkgs}] -> shared target/ (profile ${_profile})"
            COMMAND ${_goods_cmd})
    endif()
    add_custom_target(${_name}
        ${_goods_args}
        COMMAND ${CMAKE_COMMAND} -E echo
            "deploy-${_t}: configure dedicated binaryDir ${_bdir} (profile ${_profile})"
        COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir}
            -DMEDIASERVO_APP_TREE=${_t}
            ${_extra_defs}
            -DMEDIASERVO_BUILD_CRATES_LANES=OFF
            -DMEDIASERVO_DEPLOY_PROFILE=${_profile}
            -DMEDIASERVO_BRAND=${_brand}
            -DMEDIASERVO_OXMGR_BIN=${MEDIASERVO_OXMGR_BIN}
            -DCMAKE_BUILD_TYPE=${_cfg}
        ${_guard_args}
        COMMAND ${CMAKE_COMMAND} --install ${_bdir} --prefix ${_pfx}
        ${_init_args}
        COMMENT "mediaservo deploy ${_t} -> ${_pfx} (profile ${_profile}, brand '${_brand}')"
        VERBATIM)
    if(MDT_FOLDER)
        set_target_properties(${_name} PROPERTIES FOLDER "${MDT_FOLDER}")
    endif()
endfunction()

# bindings: AppTree has no bindings tree — the bindings face IS an install face
# (MediaServoBindings rows in the bindings-lane tree). Dedicated binaryDir with
# the bindings gate ON; pure install, no init legs, no guard-needed state (no
# run/oxfile in a bindings prefix; strip = no mechanism today, declared).
# submodule module callsites (gated by the lane flags as today — behavior-
# identical registration; the IDE button faces stay mediaservo-deploy-*)
if(MEDIASERVO_BUILD_CRATES_LANES)
    _mediaservo_add_deploy_target(TREE host
        PREFIX "${MEDIASERVO_DEPLOY_PREFIX_HOST}"
        BRAND "${MEDIASERVO_BRAND}"
        BUILD_ROOT "${MEDIASERVO_REPO_ROOT}/build"
        FOLDER "${MEDIASERVO_IDE_ROOT}deploy")
    _mediaservo_add_deploy_target(TREE server
        PREFIX "${MEDIASERVO_DEPLOY_PREFIX_SERVER}"
        BRAND "${MEDIASERVO_BRAND}"
        SERVER_OUT_DIR "${MEDIASERVO_SERVER_OUT_DIR}"
        STAGING build
        BUILD_ROOT "${MEDIASERVO_REPO_ROOT}/build"
        FOLDER "${MEDIASERVO_IDE_ROOT}deploy")
endif()
