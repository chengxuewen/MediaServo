# Assembly layer: MediaServo root / project / PolyOrch embed / options / add_subdirectory.
# Pure assembly — zero compilation rules here (PLAN §7.1; hosts ride supplier verbs only).
# Design + rulings: parent-repo docs/plans/polyorch-adoption/PLAN.md (F14-F22).
# Renderers here: .pc (per-domain versions) + mediaservo Config — consumed by the
# python staging step; lanes are IDE/CI build buttons over the shared target/ cache.
cmake_minimum_required(VERSION 3.25)

# Version single source: the workspace Cargo.toml (C43 — do NOT copy the five-literal
# debt; mismatch = hard configure error). First 3-component match is [workspace.package].
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml" _ws)
string(REGEX MATCH "version = \"([0-9]+\\.[0-9]+\\.[0-9]+)\"" _m "${_ws}")
if(NOT _m)
  message(FATAL_ERROR "Failed to read the Cargo.toml workspace version (single-source contract broken)")
endif()

# CXX identity required BEFORE the PolyOrch embed (PlatformSupport contract; both
# sibling consumers bled on this first — measured, not folklore).
project(mediaservo VERSION ${CMAKE_MATCH_1} LANGUAGES CXX)

# Must run before ALL add_subdirectory calls (measured sibling lesson: a late add_test
# in a subdirectory is silently dropped).
set_property(GLOBAL PROPERTY USE_FOLDERS ON)  # R7: CMP0143 keys on the 3.25 floor, not the runtime

include(cmake/MediaServoPolyOrch.cmake)
mediaservo_ensure_polyorch()   # D39 sentinel: reuse superbuild copy first-wins, else embed the gitlink pin

# Island mount defaults OFF: bindings/cxx/examples hard-FATALs unless prebuilt SDK
# outputs exist in target/ (its own PIT-189 guard) — keeping a fresh-clone configure
# green is the S1 behavior-neutral contract. Dev/CI flip it after `build bindings`.
# S2 rewires the island itself onto lanes; the mount point stays here.
option(MEDIASERVO_BUILD_EXAMPLES "Mount the C++/SDL examples aggregate (requires target/ SDK outputs)" OFF)
if(MEDIASERVO_BUILD_EXAMPLES)
  add_subdirectory(bindings/cxx/examples)
endif()

# IDE bootstrap hint (ide-bootstrap plan, C''): the workspace settings file is
# an INPUT to configure, so a failure-path hint is the only shape that can fix
# a CMake Tools first-open that picked the system cmake. One call site at the
# shared lane gate: any lane-bearing preset (bindings / crates / host / server
# / all) that needs the rust toolchain and failed the probe lands here BEFORE
# the lane functions run, so the guidance output precedes the lane FATAL.
# Silence on every green path (a workspace that configures fine is never
# touched -- the trigger itself is the wrong-toolchain detector).
if(MEDIASERVO_BUILD_BINDINGS OR MEDIASERVO_BUILD_CRATES_LANES)
  if(NOT POLYORCH_RUST_FOUND)
    polyorch_rust_setup()   # the same probe the lanes run; result reused below
  endif()
  if(NOT POLYORCH_RUST_FOUND AND COMMAND polyorch_bootstrap_hint)
    polyorch_bootstrap_hint(
        CANDIDATE_CMAKE "${MEDIASERVO_REPO_ROOT}/.pixi/envs/default/bin/cmake")
  endif()
endif()

# S3 bindings surface: lanes (shared target/ cache, deliberate F17-D exception) +
# delivery-manifest renderers (.pc per-domain / Config suite-version) — cmake/MediaServoBindings.cmake.
include(cmake/MediaServoBindings.cmake)
option(MEDIASERVO_BUILD_BINDINGS "Register bindings lanes + render delivery manifests (.pc / cmake config)" OFF)
if(MEDIASERVO_BUILD_BINDINGS)
  mediaservo_setup_bindings()
endif()

# S4 crates lanes: LIB-kind rlib lanes + per-crate test buttons + ctest twins;
# lane table single source = cmake/MediaServoCrates.cmake (reconciled against Cargo members
# by the mediaservo-gates lanes face (tools/mediaservo-gates) in all-gates). Default cache face = build-tree-local;
# shared-target preset flips lanes onto target/ and drops the clean buttons (F17-D).
include(cmake/MediaServoCrates.cmake)
option(MEDIASERVO_BUILD_CRATES_LANES "Register crates lanes (IDE build/test buttons + ctest bridge)" OFF)
if(MEDIASERVO_BUILD_CRATES_LANES)
  mediaservo_setup_crate_lanes()
endif()

# SR7 app-tree: the branded out/{host,server} bin layout as install rules (F32-C;
# cmake/MediaServoAppTree.cmake). Registered only when the CLI deploy injects
# -DMEDIASERVO_APP_TREE — every preset/IDE/gates configure registers ZERO rules
# (behavior-neutral) and no lane tree can ever re-lay an app tree through --install.
include(cmake/MediaServoAppTree.cmake)
if(MEDIASERVO_APP_TREE)
    mediaservo_setup_app_tree()
endif()
# SR6-P1 packaging: the release tars described natively (CPack TGZ only;
# cmake/MediaServoPack.cmake). Registered only when the CLI package verb injects
# -DMEDIASERVO_PKG — every other configure (presets/IDE/gates/deploy) stays
# behavior-neutral, mirroring the SR7 app-tree gate.
include(cmake/MediaServoPack.cmake)
if(MEDIASERVO_PKG)
    mediaservo_setup_pack()
endif()

# S5 web lanes: node surface over the supplier D43 pnpm support; the workspace:*
# edge (admin -> client) becomes a real dependency (PIT-194 class fix).
include(cmake/MediaServoWeb.cmake)

# K1b deploy buttons (build-env round): dedicated-binaryDir choreography targets;
# gated per-target inside (crates lanes / bindings face) — preset availability
# matrix in docs/plans/build-env-vsc-affinity/PLAN.md Knife 1b.
include(cmake/MediaServoDeployTargets.cmake)
option(MEDIASERVO_BUILD_WEB "Mount the www node lanes (web-build aggregate + per-member buttons)" OFF)
if(MEDIASERVO_BUILD_WEB)
  mediaservo_setup_web()
endif()
