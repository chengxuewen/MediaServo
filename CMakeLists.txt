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
enable_testing()

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

# S3 bindings surface: lanes (shared target/ cache, deliberate F17-D exception) +
# delivery-manifest renderers (.pc per-domain / Config suite-version) — cmake/MediaServoBindings.cmake.
include(cmake/MediaServoBindings.cmake)
option(MEDIASERVO_BUILD_BINDINGS "Register bindings lanes + render delivery manifests (.pc / cmake config)" OFF)
if(MEDIASERVO_BUILD_BINDINGS)
  mediaservo_setup_bindings()
endif()

# S4 crates lanes: LIB-kind rlib lanes + per-crate test buttons + ctest twins;
# lane table single source = cmake/MediaServoCrates.cmake (reconciled against Cargo members
# by scripts/gate-lanes.py in all-gates). Default cache face = build-tree-local;
# shared-target preset flips lanes onto target/ and drops the clean buttons (F17-D).
include(cmake/MediaServoCrates.cmake)
option(MEDIASERVO_BUILD_CRATES_LANES "Register crates lanes (IDE build/test buttons + ctest bridge)" OFF)
if(MEDIASERVO_BUILD_CRATES_LANES)
  mediaservo_setup_crate_lanes()
endif()
