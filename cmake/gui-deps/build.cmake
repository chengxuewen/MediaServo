# cmake/gui-deps/build.cmake -- the explicit verb body for the GUI dependency
# face (sr8 P3). Script mode on purpose: an independent CMake process builds
# SDL (H2 -- configure-time builds are the anti-pattern this face exists to
# kill). The only entry is ./mediaservo.sh build gui-deps.
#
# Required: -DPROFILE=<p> -DBUILD_TYPE=<cfg> -DWORK_DIR=<anchor>/<profile>[-<provider>] -DGENERATOR=<g>
# Optional: -DMAKE_PROGRAM=<abs> -DPARALLEL=<n>
#           -DPROVIDER=none|system|pixi -DPIXI_ENV=<e>          (P4; default none)
#           -DCOMPILER_C=<abs> -DCOMPILER_CXX=<abs>             (required with pixi, r4 C1)
cmake_minimum_required(VERSION 3.24)
get_filename_component(_gd_repo "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)
if(NOT PROFILE OR NOT BUILD_TYPE OR NOT WORK_DIR OR NOT GENERATOR)
    message(FATAL_ERROR "gui-deps build: PROFILE, BUILD_TYPE, WORK_DIR and GENERATOR are required (script mode has no defaults)")
endif()
include("${_gd_repo}/PolyOrch/cmake/PolyOrchThirdPartyHelpers.cmake")
# B8 archive-source policy (P/S mode split; ONE point shared with the
# configure-time imgui fetch in MediaServoGuiDeps.cmake):
# parent-shell env -> LOCAL_ARCHIVE_DIR override; no env but a sibling
# gui-deps archive layer -> structural P mode (IDE face); neither ->
# DOWNLOAD on into the cache anchor (first build fetches, every later
# build replays offline).
include("${_gd_repo}/cmake/gui-deps/source-mode.cmake")
mediaservo_gui_deps_source_mode("${_gd_repo}" _gd_local _gd_dl_args)

# pkg-config lockdown (moved from examples/cxx with the SDL build, PIT-199
# four-gate behavior #3): conda's pkg-config bakes its own dirs into the
# default search set and env stripping alone cannot block it (liburing/dbus
# leaked back into SDL's probes, focal incident 2026-09-28). A caller-set
# PKG_CONFIG_LIBDIR is honored. Script mode has no CMAKE_SYSTEM_NAME -- the
# HOST spelling is the honest one here.
if(CMAKE_HOST_SYSTEM_NAME STREQUAL "Linux" AND "$ENV{PKG_CONFIG_LIBDIR}" STREQUAL "")
    set(_gd_multiarch "")
    if(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(x86_64|amd64)$")
        set(_gd_multiarch "/usr/lib/x86_64-linux-gnu/pkgconfig")
    elseif(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64)$")
        set(_gd_multiarch "/usr/lib/aarch64-linux-gnu/pkgconfig")
    endif()
    set(_gd_pc "")
    foreach(_d ${_gd_multiarch} /usr/lib/pkgconfig /usr/share/pkgconfig)
        string(APPEND _gd_pc "${_d}:")
    endforeach()
    string(REGEX REPLACE ":$" "" _gd_pc "${_gd_pc}")
    set(ENV{PKG_CONFIG_LIBDIR} "${_gd_pc}")
    message(STATUS "gui-deps build: pkg-config pinned to the system set (${_gd_pc})")
endif()

# ---- dependency provider leg (P4; F3=A: implemented, never the default) ----
set(_gd_provider "none")
if(PROVIDER)
    set(_gd_provider "${PROVIDER}")
endif()
set(_gd_extra "")
if(_gd_provider STREQUAL "pixi")
    if(NOT PIXI_ENV)
        message(FATAL_ERROR "gui-deps build: PROVIDER=pixi requires PIXI_ENV (the isolated environment declared in pixi.toml)")
    endif()
    if(NOT COMPILER_C OR NOT COMPILER_CXX)
        message(FATAL_ERROR "gui-deps build: PROVIDER=pixi requires COMPILER_C and COMPILER_CXX -- the provider never lets the child discover a compiler (r4 C1); the verb passes the system pair (/usr/bin/cc, /usr/bin/c++)")
    endif()
    # the pixi face context: the binary plus the manifest that owns the
    # gui-deps environment; env paths then resolve the installed prefix
    include("${_gd_repo}/PolyOrch/cmake/PolyOrchPixiHelpers.cmake")
    polyorch_pixi_setup(MANIFEST "${_gd_repo}/pixi.toml")
    set(_gd_extra "-DCMAKE_C_COMPILER=${COMPILER_C}" "-DCMAKE_CXX_COMPILER=${COMPILER_CXX}")
    message(STATUS "gui-deps build: provider=pixi env=${PIXI_ENV} compilers=${COMPILER_C},${COMPILER_CXX}")
endif()

polyorch_thirdparty_recipe(RECIPE "${_gd_repo}/cmake/gui-deps/recipes/sdl3.json"
    WORK_DIR "${WORK_DIR}" BUILD_TYPE "${BUILD_TYPE}" PROFILE "${PROFILE}"
    GENERATOR "${GENERATOR}" MAKE_PROGRAM "${MAKE_PROGRAM}" PARALLEL "${PARALLEL}"
    DEPENDENCY_PROVIDER "${_gd_provider}" PIXI_ENVIRONMENT "${PIXI_ENV}"
    CMAKE_ARGS ${_gd_extra}
    ${_gd_dl_args}
    LOCAL_ARCHIVE_DIR "${_gd_local}"
    INSTALL_DIR_OUT _gd_install FINGERPRINT_OUT _gd_key MANIFEST_OUT _gd_mf)
if(NOT EXISTS "${_gd_install}/lib/cmake/SDL3/SDL3Config.cmake")
    message(FATAL_ERROR "gui-deps build: SDL3 install finished but '${_gd_install}/lib/cmake/SDL3/SDL3Config.cmake' is missing -- the upstream install layout changed")
endif()
message(STATUS "gui-deps: sdl3 ${PROFILE}/${BUILD_TYPE} ready at ${_gd_install} (key ${_gd_key})")
