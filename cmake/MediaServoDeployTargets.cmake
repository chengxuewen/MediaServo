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
    set(GUARD_PREFIX "${_prefix}")
    set(GUARD_BRAND "${_brand}")
    if(_brand)
        set(GUARD_IS_BRANDED 1)
    else()
        set(GUARD_IS_BRANDED 0)
    endif()
    set(GUARD_PROFILE "${MEDIASERVO_DEPLOY_PROFILE}")
    configure_file("${CMAKE_CURRENT_LIST_DIR}/deploy-guard.cmake.in"
                   "${_out}" @ONLY)
endfunction()

# ---- target registration ----

function(_mediaservo_deploy_target _t)
    cmake_parse_arguments(DT "" "TREE;GATE" "EXTRA_DEFS" ${ARGN})
    if(NOT ${${DT_GATE}})
        return()
    endif()
    if(DT_TREE STREQUAL "host")
        set(_pfx "${MEDIASERVO_DEPLOY_PREFIX_HOST}")
    else()
        set(_pfx "${MEDIASERVO_DEPLOY_PREFIX_SERVER}")
        set(_server_defs -DMEDIASERVO_SERVER_OUT_DIR=${MEDIASERVO_SERVER_OUT_DIR}
                        -DMEDIASERVO_APP_STAGING=build)
    endif()
    set(_bdir "${CMAKE_BINARY_DIR}/../deploy-${_t}")
    set(_profile ${MEDIASERVO_DEPLOY_PROFILE})
    if(_profile STREQUAL "Release")
        set(_cfg Release)
    else()
        set(_cfg Debug)
    endif()
    # per-target guard script in the MAIN binaryDir (configure_file embeds
    # prefix/brand as @ONLY constants)
    _mediaservo_deploy_guard_script(
        "${CMAKE_BINARY_DIR}/deploy-${_t}-guard.cmake" "${_pfx}" "${MEDIASERVO_BRAND}")
    add_custom_target(mediaservo-deploy-${_t}
        COMMAND ${CMAKE_COMMAND} -E echo
            "deploy-${_t}: configure dedicated binaryDir ${_bdir} (profile ${_profile})"
        COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir}
            -DMEDIASERVO_APP_TREE=${DT_TREE}
            $<$<BOOL:${_server_defs}>:${_server_defs}>
            -DMEDIASERVO_BUILD_CRATES_LANES=ON
            -DMEDIASERVO_DEPLOY_PROFILE=${_profile}
            -DMEDIASERVO_BRAND=${MEDIASERVO_BRAND}
            -DMEDIASERVO_OXMGR_BIN=${MEDIASERVO_OXMGR_BIN}
            -DCMAKE_BUILD_TYPE=${_cfg}
        COMMAND ${CMAKE_COMMAND} --build ${_bdir}
        COMMAND ${CMAKE_COMMAND}
            -DMEDIASERVO_DEPLOY_GUARD=1
            -DMEDIASERVO_DEPLOY_GUARD_PREFIX=${_pfx}
            -DMEDIASERVO_DEPLOY_GUARD_BRAND=${MEDIASERVO_BRAND}
            -P ${CMAKE_BINARY_DIR}/deploy-${_t}-guard.cmake
        COMMAND ${CMAKE_COMMAND} --install ${_bdir} --prefix ${_pfx}
        COMMENT "mediaservo deploy ${_t} -> ${_pfx} (profile ${_profile}, brand '${MEDIASERVO_BRAND}')"
        VERBATIM)
endfunction()

_mediaservo_deploy_target(host TREE host GATE MEDIASERVO_BUILD_CRATES_LANES)
_mediaservo_deploy_target(server TREE server GATE MEDIASERVO_BUILD_CRATES_LANES)

# bindings: AppTree has no bindings tree — the bindings face IS an install face
# (MediaServoBindings rows in the bindings-lane tree). Dedicated binaryDir with
# the bindings gate ON; pure install, no init legs, no guard-needed state (no
# run/oxfile in a bindings prefix; strip = no mechanism today, declared).
if(MEDIASERVO_BUILD_BINDINGS)
    set(_bdir_b "${CMAKE_BINARY_DIR}/../deploy-bindings")
    set(_cfg_b ${MEDIASERVO_DEPLOY_PROFILE})
    if(_cfg_b STREQUAL "Release")
        set(_cfg_b Release)
    else()
        set(_cfg_b Debug)
    endif()
    add_custom_target(mediaservo-deploy-bindings
        COMMAND ${CMAKE_COMMAND} -S ${MEDIASERVO_REPO_ROOT} -B ${_bdir_b}
            -DMEDIASERVO_BUILD_BINDINGS=ON
            -DCMAKE_BUILD_TYPE=${_cfg_b}
        COMMAND ${CMAKE_COMMAND} --build ${_bdir_b}
            --target mediaservo-bindings-manifest
        COMMAND ${CMAKE_COMMAND} --install ${_bdir_b} --prefix ${MEDIASERVO_DEPLOY_PREFIX_HOST}
        COMMENT "mediaservo deploy bindings -> ${MEDIASERVO_DEPLOY_PREFIX_HOST} (bindings manifest + delivery rows)"
        VERBATIM)
endif()
