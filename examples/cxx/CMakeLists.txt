# examples/cxx — cxx band root of the top-level examples/ aggregator (plan directory-artifacts-convergence batch ①; was bindings/cxx/examples, p3-gui-viewer W0)
#
# The single CLI entry point is `./mediaservo.sh build example [name...]` (G7). Since batch ① the bridge configures the
# REPO ROOT (link faces live there); a bare `cmake -S . -B build` of this band no longer resolves mediaservo::<sdk>
# (the pre-faces standalone era passed control_demo only a raw .so path — the branding retired that bonus).
#
# Contracts (PLAN §3/§6):
# - Directory name == target name == artifact name (the three-in-one G4): the subdirectory scan follows exactly the same literal rule as this file's
# file(GLOB */CMakeLists.txt) (F-A-6); the CLI `list example` reuses the same glob.
# - MEDIASERVO_SDK_DIR = the build-c output directory (containing libmediaservo_*.so). Missing / no .so →
# FATAL_ERROR with a message pointing at build-c (the compile-surface equivalent of the PIT-189 deployment-freshness lesson).
# - GUI dependencies = the gui-deps face (sr8 P3): archives are hash-pinned, SDL3 is built by the
# explicit `./mediaservo.sh build gui-deps` verb and consumed through its upstream SDL3Config.cmake;
# cmake/gui-deps/ is the single definition point (recipes + patch scripts + the ImGui wrapper).
# The ledger is 3rdparty/PROVENANCE-gui-deps.md. A missing install prefix is a FATAL naming the verb.
# - Env constitution: the F25 gate (below) SELF-CHECKS the shell — an activated conda/pixi
#   configure with the mount ON warns + force-OFFs; plain-shell -D or the CLI sanitized
#   bridge are the two legit routes. pkg-config lockdown lives here regardless.

cmake_minimum_required(VERSION 3.24)
project(mediaservo_examples LANGUAGES C CXX)

enable_testing() # prerequisite for subdirectory add_test to take effect (the core test surface)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)   # load-bearing wall (plan batch ①/P3): band sits at depth 2 under the repo root

# -- SDK delivery directory (build-c prerequisite) --
set(MEDIASERVO_SDK_DIR "${REPO_ROOT}/target/debug" CACHE PATH
    "build-c output directory (where libmediaservo_*.so lives)")
if(NOT EXISTS "${MEDIASERVO_SDK_DIR}/libmediaservo_client.so")
    message(FATAL_ERROR
        "no libmediaservo_client.so inside MEDIASERVO_SDK_DIR='${MEDIASERVO_SDK_DIR}' —"
        " run ./mediaservo.sh build-c (or pixi run build-c) before configuring (PIT-189)")
endif()

# -- F25 environment-eligibility gate (system constitution self-check) --
# The C++/SDL island must compile against the SYSTEM toolchain (desktop runtime alignment,
# PIT-199/204 lesson). Compiler-direct-read vars (CPATH/LIBRARY_PATH/COMPILER_PATH/...) cannot
# be countered by cache variables — an activated conda/pixi shell must never configure this
# tree. On a dirty shell with the mount ON: WARN + force the option OFF (loud degrade; never
# silently misbuild). Legit routes: the CLI `mediaservo.sh build example` (sanitized env
# bridge) or a plain shell with -DMEDIASERVO_BUILD_EXAMPLES=ON.
set(_ms_env_dirty FALSE)
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    foreach(_v CONDA_PREFIX CPATH C_INCLUDE_PATH CPLUS_INCLUDE_PATH LIBRARY_PATH COMPILER_PATH)
        if(DEFINED ENV{${_v}} AND NOT "$ENV{${_v}}" STREQUAL "")
            set(_ms_env_dirty TRUE)
        endif()
    endforeach()
    if(NOT _ms_env_dirty AND "$ENV{PATH}" MATCHES "\\.pixi/envs/")
        set(_ms_env_dirty TRUE)   # activated PATH drifts compiler/pkg-config discovery
    endif()
    foreach(_c "$ENV{CC}" "$ENV{CXX}")
        if(_c MATCHES "conda|\\.pixi/envs")
            set(_ms_env_dirty TRUE)   # conda compiler pin would hijack the system rule
        endif()
    endforeach()
endif()
if(MEDIASERVO_BUILD_EXAMPLES AND _ms_env_dirty)
    message(WARNING
        "examples cxx band: activated conda/pixi shell detected — band SKIPPED for this "
        "configure (system-toolchain constitution, F25). Build via `./mediaservo.sh build example` "
        "(sanitized bridge) or re-run from a plain shell.")
    # batch ①/④ discipline: skip MY band only — flipping the family cache here would
    # retroactively drop the sibling rs/py material on the next reconfigure
    return()
endif()


# -- link-face consumption (plan bindings-faces P5, ruling 7-B) ------------------
# The hand-rolled SDK shell (mediaservo_client_sdk, retired here) was the pre-face
# shape: its include×4 + .so path + rpath triple now lives in mediaservo::client
# (MediaServoBindings.cmake P2/P3) — the single authority, same name in tree and
# install states (iron law 7). Forward reference of the later-defined ALIAS is the
# measured pattern (P1④ probe: configure+generate green; the island mounts at the
# root's :50 before the bindings setup at :74).
if(NOT MEDIASERVO_BUILD_BINDINGS)
    message(FATAL_ERROR
        "examples island requires MEDIASERVO_BUILD_BINDINGS=ON — the link faces"
        " (mediaservo::<sdk>) are registered by the bindings lanes (bindings-faces"
        " ruling 7-B cross-gate check; gate-variable form per R4-2: a TARGET-existence"
        " test here could never pass, the setup runs after this mount)")
endif()

# Island-private conda libstdc++ patch (ruling 6-A: environment-specific, consumer-
# side only — never rides the public face; ruling 7-B moved it off the old shell).
# The SDK is built with the conda toolchain (GLIBCXX_3.4.29/3.4.30 new symbols): when the examples use the system
# compiler chain (the focal header mix fix, 2026-09-28), linking needs rpath-link to see the conda
# libstdc++ to resolve versioned symbols; at runtime the .so's own RPATH backstops it, and the conda compiler also tolerates this flag.
add_library(mediaservo-examples-toolchain-patch INTERFACE)
set(_ms_conda_lib "${REPO_ROOT}/.pixi/envs/default/lib")
if(EXISTS "${_ms_conda_lib}/libstdc++.so.6")
    # -L makes the driver-appended -lstdc++ resolve to the conda version (full versioned symbols; first -L wins);
    # rpath-link lets ld resolve the .so's NEEDED chain; rpath keeps the runtime loading the conda libstdc++
    # (system gcc-10 has 3.4.28 without 3.4.29/30 → a runtime version-not-found).
    target_link_options(mediaservo-examples-toolchain-patch INTERFACE
        "-L${_ms_conda_lib}"
        "-Wl,-rpath-link,${_ms_conda_lib}"
        "-Wl,-rpath,${_ms_conda_lib}")
endif()

# (the pkg-config lockdown that used to live here moved with the SDL build into
#  cmake/gui-deps/build.cmake: the SDL configure is an INDEPENDENT process now,
#  so its env constitution must ride that process, not this band's configure)

# -- GUI dependencies (sr8 P3): ONE definition point = cmake/gui-deps/ -------
# SDL3 is built/installed by the explicit verb (./mediaservo.sh build
# gui-deps) -- configure never builds dependencies (H2; an IDE reconfigure
# must not trigger minutes of compilation). This call imports the upstream
# SDL3Config.cmake as the upstream static target, exposes MediaServoGui::SDL3, and
# defines the mediaservo-imgui source wrapper (F8, consumer-compiled).
# A missing install prefix is a FATAL naming the verb with the exact
# --profile/--release mirror. Recipes/patches hash-pin the archives; the
# ledger is 3rdparty/PROVENANCE-gui-deps.md.
if(NOT MEDIASERVO_REPO_ROOT)
    set(MEDIASERVO_REPO_ROOT "${REPO_ROOT}")   # mounted-tree fallback; the shim sets it in every real configure
endif()
include("${REPO_ROOT}/cmake/MediaServoGuiDeps.cmake")
mediaservo_gui_deps()




# -- Example aggregation: subdirectory scan with the same literal rule (same source as the CLI list, F-A-6) --
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")

file(GLOB EXAMPLE_DIRS RELATIVE "${CMAKE_CURRENT_LIST_DIR}" CONFIGURE_DEPENDS
     "${CMAKE_CURRENT_LIST_DIR}/*/CMakeLists.txt")
set(_known_targets "")
foreach(entry ${EXAMPLE_DIRS})
    get_filename_component(dir "${entry}" DIRECTORY)
    add_subdirectory("${dir}")
    # C-D9 G4 upgrade: dir keeps the plain name; target/artifact carry the brand
    # (dir+prefix == target == artifact; the CLI maps user names the same way).
    set(_tgt "mediaservo-example-${dir}")
    list(APPEND _known_targets "${_tgt}")
    # The three-in-one gate (G4, configure-time assertion before any compilation).
    if(NOT TARGET "${_tgt}")
        message(FATAL_ERROR "example directory '${dir}/' lacks target '${_tgt}' (a G4 dir+prefix==target violation)")
    endif()
    # IDE run button (S2, supplier surface). Standalone band configure has no PolyOrch loaded →
    # COMMAND guard keeps the compatibility bonus valid. GUI flavor = the supplier's exec-time
    # display-probe chain (no display vars + a local X socket → DISPLAY=:0); CI/ctest never
    # passes through it. CWD pinned to the repo root (plan C-D4=A: imgui.ini lands there no
    # matter where the build dir is; unset = generator build-dir default). The supplier ships
    # no build edge by design — we wire our own.
    if(COMMAND polyorch_example_run)
        get_target_property(_et "${_tgt}" TYPE)
        if(_et STREQUAL "EXECUTABLE")
            polyorch_example_run(TARGET "${_tgt}" GUI WORKING_DIRECTORY "${REPO_ROOT}" FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx/${dir}")
            add_dependencies("${_tgt}-run" "${_tgt}")
        endif()
    endif()
endforeach()
message(STATUS "mediaservo examples: ${_known_targets}")
