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
#   MEDIASERVO_DEPLOY_PREFIX_HOST/SERVER  default ${repo}/../out/<t>, env
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
    set(MEDIASERVO_DEPLOY_PREFIX_HOST "${MEDIASERVO_REPO_ROOT}/../out/host" CACHE PATH
        "deploy target prefix" FORCE)
endif()
if(DEFINED ENV{MSRTC_OUT_ROOT} AND NOT MEDIASERVO_DEPLOY_PREFIX_SERVER)
    set(MEDIASERVO_DEPLOY_PREFIX_SERVER "$ENV{MSRTC_OUT_ROOT}/server" CACHE PATH
        "deploy target prefix (env fallback)" FORCE)
endif()
if(NOT MEDIASERVO_DEPLOY_PREFIX_SERVER)
    set(MEDIASERVO_DEPLOY_PREFIX_SERVER "${MEDIASERVO_REPO_ROOT}/../out/server" CACHE PATH
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

# bindings: AppTree has no bindings tree — the bindings face IS an install face
# (MediaServoBindings rows in the bindings-lane tree). Dedicated binaryDir with
# the bindings gate ON; pure install, no init legs, no guard-needed state (no
# run/oxfile in a bindings prefix; strip = no mechanism today, declared).
# R6 NOTE: this face does NOT ride _mediaservo_add_deploy_target — the shared
# function drives the AppTree choreography (lanes+guard+init), which is the
# HOST/SERVER shape; the bindings target = configure(bindings gate) -> build
# manifest -> install. Shape mismatch is by design, not an oversight.
if(MEDIASERVO_BUILD_BINDINGS)
    set(_bdir_b "${CMAKE_BINARY_DIR}/../deploy-bindings")
    set(_cfg_b "${MEDIASERVO_DEPLOY_PROFILE}")
    if(_cfg_b STREQUAL "Release")
        set(_cfg_b Release)
    else()
        set(_cfg_b Debug)
    endif()
    add_custom_target(mediaservo-deploy-bindings
        COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir_b}
            -DMEDIASERVO_BUILD_BINDINGS=ON -DCMAKE_BUILD_TYPE=${_cfg_b}
        COMMAND ${CMAKE_COMMAND} --build ${_bdir_b}
            --target mediaservo-bindings-manifest
        COMMAND ${CMAKE_COMMAND} --install ${_bdir_b} --prefix ${MEDIASERVO_DEPLOY_PREFIX_HOST}
        COMMENT "mediaservo deploy bindings -> ${MEDIASERVO_DEPLOY_PREFIX_HOST} (bindings manifest + delivery rows)"
        VERBATIM)
    set_target_properties(mediaservo-deploy-bindings PROPERTIES FOLDER "deploy")
endif()
if(NOT MEDIASERVO_DEPLOY_PREFIX_HOST)
    set(MEDIASERVO_DEPLOY_PREFIX_HOST "${MEDIASERVO_REPO_ROOT}/../out/host" CACHE PATH
        "deploy target prefix" FORCE)
endif()
if(DEFINED ENV{MSRTC_OUT_ROOT} AND NOT MEDIASERVO_DEPLOY_PREFIX_SERVER)
    set(MEDIASERVO_DEPLOY_PREFIX_SERVER "$ENV{MSRTC_OUT_ROOT}/server" CACHE PATH
        "deploy target prefix (env fallback)" FORCE)
endif()
if(NOT MEDIASERVO_DEPLOY_PREFIX_SERVER)
    set(MEDIASERVO_DEPLOY_PREFIX_SERVER "${MEDIASERVO_REPO_ROOT}/../out/server" CACHE PATH
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
    cmake_parse_arguments(MDT "" "TREE;NAME;PREFIX;BRAND;FOLDER;SERVER_OUT_DIR;STAGING;INIT" "" ${ARGN})
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
    set(_bdir "${CMAKE_BINARY_DIR}/../deploy-${_t}")
    set(_profile "${MEDIASERVO_DEPLOY_PROFILE}")
    if(_profile STREQUAL "Release")
        set(_cfg Release)
    else()
        set(_cfg Debug)
    endif()
    set(_extra_defs "")
    set(_extra_commands "")
    if(NOT _t STREQUAL "bindings")
        _mediaservo_deploy_guard_script(
            "${CMAKE_BINARY_DIR}/deploy-${_name}-guard.cmake" "${_pfx}" "${_brand}")
        set(_guard_args
            COMMAND ${CMAKE_COMMAND}
                -DMEDIASERVO_DEPLOY_GUARD=1
                -DMEDIASERVO_DEPLOY_GUARD_PREFIX=${_pfx}
                -DMEDIASERVO_DEPLOY_GUARD_BRAND=${_brand}
                -DMEDIASERVO_DEPLOY_GUARD_MODE=${MEDIASERVO_DEPLOY_GUARD_MODE}
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
    add_custom_target(${_name}
        COMMAND ${CMAKE_COMMAND} -E echo
            "deploy-${_t}: configure dedicated binaryDir ${_bdir} (profile ${_profile})"
        COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir}
            -DMEDIASERVO_APP_TREE=${_t}
            ${_extra_defs}
            -DMEDIASERVO_BUILD_CRATES_LANES=ON
            -DMEDIASERVO_DEPLOY_PROFILE=${_profile}
            -DMEDIASERVO_BRAND=${_brand}
            -DMEDIASERVO_OXMGR_BIN=${MEDIASERVO_OXMGR_BIN}
            -DCMAKE_BUILD_TYPE=${_cfg}
        COMMAND ${CMAKE_COMMAND} --build ${_bdir}
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
        BRAND "${MEDIASERVO_BRAND}")
    _mediaservo_add_deploy_target(TREE server
        PREFIX "${MEDIASERVO_DEPLOY_PREFIX_SERVER}"
        BRAND "${MEDIASERVO_BRAND}"
        SERVER_OUT_DIR "${MEDIASERVO_SERVER_OUT_DIR}"
        STAGING build)
endif()
