# examples — band aggregator (plan directory-artifacts-convergence, ruling C-D1-B)
#
# Mounted from the repo root behind MEDIASERVO_BUILD_EXAMPLES (the root keeps the
# option and the F25/PIT-189 story note). One add_subdirectory per language band;
# each band root stays STANDALONE-configurable too — the CLI bridge -S's straight
# into the band, a contract the island had before aggregation and keeps.
# IDE groups mirror the disk: examples/<band> (C-D3; ${V} prefix rides the
# submodule's MEDIASERVO_IDE_ROOT, plan ide-folder-hierarchy mechanism).
#
# cxx  — GUI/SDK examples island (F25 system-toolchain constitution + PIT-189
#        SDK-dir + cross-gate BINDINGS, all in the band root).
# c    — batch ②: crate-embedded example material (plan C-D2).
# py   — batch ③: wheel-out examples relocation, C-D7 safe-valve applies.
# rs   — batch ④: workspace-member example package + PolyOrch D40 EXAMPLE face
#        (C-D6=B/C-D10=B'). The CMake face here inherits this family gate;
#        pure `cargo run -p examples --example <n>` consumption never does.

add_subdirectory(cxx)
