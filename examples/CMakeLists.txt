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
#        SDK-dir + cross-gate BINDINGS, all in the band root). Crate usage material
#        (vehicle_{field,link,deck}.cpp + standalone ros2_node/) rides under
#        cxx/crate-examples/ — one level deeper, deliberately invisible to the band
#        G4 glob (*/CMakeLists.txt); compiled by e2e/docs paths, not CMake targets.
# c    — examples-ide-unification ruling 3=B: CMake-ized (mirror of the cxx
#        band minus F25/gui-deps/toolchain-patch; pure C needs none, so these
#        targets appear in the preset tree too). The e2e gcc script stays the
#        pure-gcc validation path; this face adds IDE buttons + F5 rows.
# py   — batch ③ LANDED (usage examples at examples/py/, never wheel content;
#        the CLI py lane still execs them directly) + ruling 4: F5 rows via
#        the python face (SCRIPT-based, no executable target needed).
# rs   — batch ④ LANDED: workspace-member example package + PolyOrch D40 EXAMPLE face
#        (C-D6=B/C-D10=B', 8 lanes all live). The CMake face here inherits this family
#        gate; pure `cargo run -p examples --example <n>` consumption never does.
#        NOTE (measured, ruling-4 round): the rs lanes' F5 rows also ride the
#        cache-level knob (PolyOrch_VSCODE_DEBUG_ALL in the bridge cfgargs /
#        the all-examples preset) — see the py band header for the gate
#        layering; the local set() inside mediaservo_setup_crate_lanes only
#        drives spec REGISTRATION, never the end-of-configure generator.

# rs band face (C-D10=B' six-piece set, VE parity). Soft-degrade posture = the C23
# crates precedent: no rust toolchain → STATUS skip, zero invented targets. Mount
# order note (P1⑦ cascade ruling, settled in batch ④): the cxx band's hard FATALs
# (PIT-189 SDK-dir / cross-gate BINDINGS) abort the whole configure, so rs registers
# FIRST and its own gate is the soft one; the cxx F25 probe now skips only its band
# (return(), no cache flip) — a dirty shell costs the cxx buttons, never the rs face.
# Pure-cargo rs users never touch CMake at all.
get_filename_component(_EX_ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)   # the submodule repo root
set(_rs_items basic pull_observe push_observe vehicle_push square-gen square-gen-egui viewer webrtc_loopback_egui)

# disk<->table two-way audit (registry = the Cargo.toml [[example]] list; drift here
# is build-time red for cargo, configure-time FATAL for the CMake face — VE discipline)
file(GLOB _rsf CONFIGURE_DEPENDS "${CMAKE_CURRENT_LIST_DIR}/rs/*.rs")
foreach(_f ${_rsf})
    get_filename_component(_n ${_f} NAME_WE)
    if(NOT _n IN_LIST _rs_items)
        message(FATAL_ERROR "examples/rs/${_n}.rs is not in the registry (stale disk; comment-style disabling forbidden)")
    endif()
endforeach()

if(NOT POLYORCH_RUST_FOUND)
    polyorch_rust_setup()
endif()
set(_rs_mediators "")
if(POLYORCH_RUST_FOUND)
    foreach(_n IN LISTS _rs_items)
        if(NOT EXISTS "${CMAKE_CURRENT_LIST_DIR}/rs/${_n}.rs")
            message(FATAL_ERROR "registry entry ${_n} has no file on disk (examples/rs/${_n}.rs)")
        endif()
        polyorch_rust_build(TARGET mediaservo-example-${_n}
            PACKAGE examples CRATE ${_n}
            EXAMPLE MANIFEST "${CMAKE_CURRENT_LIST_DIR}/rs/Cargo.toml"
            FOLDER "${MEDIASERVO_IDE_ROOT}examples/rs")
        polyorch_rust_ide_mount(TARGET mediaservo-example-${_n}-build
            MANIFEST "${CMAKE_CURRENT_LIST_DIR}/rs/Cargo.toml"
            CRATE ${_n}
            FOLDER "${MEDIASERVO_IDE_ROOT}examples/rs")
        list(APPEND _rs_mediators mediaservo-example-${_n}-build)
        if(COMMAND polyorch_rust_run)
            # GUI flavor only for the display-backed trio; the rest run console.
            # The rust run face, NOT polyorch_example_run: the run face is what
            # owns the F5 (CodeLLDB) debug registration (ruling 4; measured gap —
            # example_run is the plain-exec face and registers nothing). It also
            # wires the -build edge itself, so no add_dependencies here.
            if(_n MATCHES "egui|^viewer$")
                set(_rs_gui GUI)
            else()
                set(_rs_gui "")
            endif()
            polyorch_rust_run(TARGET mediaservo-example-${_n} ${_rs_gui}
                WORKING_DIRECTORY "${_EX_ROOT}" FOLDER "${MEDIASERVO_IDE_ROOT}examples/rs")
        endif()
    endforeach()

    # ctest roster (P6 batch-4 ruling): ONLY self-terminating, server-free, console
    # examples — the field/client trio needs a running signaling server (its old
    # usage headers said so; e2e-compat drives them via cargo/test paths instead).
    add_test(NAME example_square-gen
        COMMAND $<TARGET_FILE:mediaservo-example-square-gen>)
    set_tests_properties(example_square-gen PROPERTIES LABELS "example;rs;headless"
        WORKING_DIRECTORY "${_EX_ROOT}" TIMEOUT 60)

    # family aggregate + clean. The clean leg goes through the supplier's own face:
    # polyorch_rust_clean(PACKAGE+MANIFEST) is `cargo clean --package examples
    # --manifest-path ... --target-dir <face's isolated .cargo-target>` — a hand-rolled
    # `cargo clean -p examples` would sweep the repo target/ cache instead and leave
    # these very artifacts behind. Disk home exists (examples/rs) -> FOLDER rides it.
    add_custom_target(mediaservo-rs-build ALL
        DEPENDS ${_rs_mediators}
        COMMENT "cargo build examples package (8 [[example]] lanes)")
    set_target_properties(mediaservo-rs-build PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/rs")
    polyorch_rust_clean(NAME mediaservo-rs-clean PACKAGE examples
        MANIFEST "${CMAKE_CURRENT_LIST_DIR}/rs/Cargo.toml"
        FOLDER "${MEDIASERVO_IDE_ROOT}examples/rs")
else()
    message(STATUS "examples rs band: no cargo/rustc found (C23 posture) — rs CMake face SKIPPED; pure cargo consumption unaffected")
endif()

add_subdirectory(c)
add_subdirectory(py)
add_subdirectory(cxx)
