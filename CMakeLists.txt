# Assembly layer: MediaServo root / project / PolyOrch embed / options / add_subdirectory.
# Pure assembly — zero compilation rules here (PLAN §7.1; hosts ride supplier verbs only).
# Design + rulings: parent-repo docs/plans/polyorch-adoption/PLAN.md (F14-F22).
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
