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
# in a subdirectory is silently dropped). The parent also carries its own copy;
# this one makes the STANDALONE tree test-capable — the examples CLI bridge now
# configures this root, so `test example` ctest needs it (plan
# directory-artifacts-convergence batch ①).
enable_testing()
set_property(GLOBAL PROPERTY USE_FOLDERS ON)  # R7: CMP0143 keys on the 3.25 floor, not the runtime

# IDE folder-root prefix (plan ide-folder-hierarchy, rulings 1/2 + rules 1/7): the ONE
# authority for the mediaservo/ subtree grouping, defined before every lane face and
# every add_subdirectory (the island mount below included). Trailing-slash convention:
# embedded = "mediaservo/", standalone = "" (disk-mirror names stay verbatim, zero
# regression). FATAL guard: a future cmake floor below 3.21 would leave
# PROJECT_IS_TOP_LEVEL undefined and silently mis-group the whole tree — make it loud.
if(NOT DEFINED PROJECT_IS_TOP_LEVEL)
  message(FATAL_ERROR
    "PROJECT_IS_TOP_LEVEL is undefined — the cmake_minimum_required floor must stay "
    ">= 3.21 for IDE grouping (plan ide-folder-hierarchy rule 7).")
endif()
if(PROJECT_IS_TOP_LEVEL)
  set(MEDIASERVO_IDE_ROOT "")
else()
  set(MEDIASERVO_IDE_ROOT "mediaservo/")
endif()
message(STATUS "MediaServo IDE root prefix: '${MEDIASERVO_IDE_ROOT}' (top-level=${PROJECT_IS_TOP_LEVEL})")

include(cmake/MediaServoPolyOrch.cmake)
mediaservo_ensure_polyorch()   # D39 sentinel: reuse superbuild copy first-wins, else embed the gitlink pin

# Warm-tree self-heal for relocated cargo sources (plan directory-artifacts-convergence
# batch ④). PolyOrch caches every manifest+crate IDE source list as an INTERNAL cache
# entry (PolyOrchRustHelpers.cmake:1605) and reuses it whenever the key is already
# DEFINED (:1521) — the key carries no mtime stamp. Moving a cargo target's src_path
# (crates/*/examples -> examples/rs) therefore leaves a warm tree listing files that no
# longer exist, and generate dies with "Cannot find source file" on every later
# reconfigure until the tree is wiped. Drop ONLY the entries that name a missing file:
# healthy trees keep the vendor's cache (and its configure speed) untouched, and the
# metadata probe re-runs exactly where it must. Runs before any mounting (examples band,
# bindings/crates lanes) because those are the readers.
get_cmake_property(_meta_vars CACHE_VARIABLES)
foreach(_v IN LISTS _meta_vars)
  if(NOT _v MATCHES "^_polyorch_meta_srcs_")
    continue()
  endif()
  set(_vals "${${_v}}")
  foreach(_f IN LISTS _vals)
    if(NOT EXISTS "${_f}")
      message(STATUS "rust ide sources: stale cache entry for '${_v}' (${_f} is gone) — re-probing cargo metadata")
      unset("${_v}" CACHE)
      break()
    endif()
  endforeach()
endforeach()

# Island mount defaults OFF: the examples/cxx band hard-FATALs unless prebuilt SDK
# outputs exist in target/ (its own PIT-189 guard) — keeping a fresh-clone configure
# green is the S1 behavior-neutral contract. Dev/CI flip it after `build bindings`.
# S2 rewires the island itself onto lanes; the mount point stays here (plan
# directory-artifacts-convergence batch ①: the top-level examples/ aggregator owns
# the per-band mounting now — C-D1-B).
option(MEDIASERVO_BUILD_EXAMPLES "Mount the examples family via examples/ (cxx band: SDL/GUI, requires target/ SDK outputs)" OFF)
if(MEDIASERVO_BUILD_EXAMPLES)
  add_subdirectory(examples)
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
