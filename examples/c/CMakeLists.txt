# examples/c — C band of the top-level examples/ aggregator
# (examples-ide-unification ruling 3=B: CMake-ized mirroring the cxx band
# machinery minus the three pieces pure C does not need — NO F25 env gate,
# NO gui-deps, NO toolchain patch. Pure C compiles fine under either
# toolchain constitution, so unlike cxx these targets DO appear in the
# CMake Tools preset tree. The e2e gcc script stays untouched as the
# pure-gcc validation path; this face adds IDE buttons + F5 rows, it does
# not replace e2e.) C60 note: the band-aggregator + SDK-guard shape is a
# known 2nd instance (registered; abstraction triggers on a 3rd band).
#
# Contracts (mirrored from cxx):
# - Directory name == target suffix == artifact name (G4, three-in-one);
#   the CLI `build example` verb does not know the c band yet (open item,
#   examples-ide-unification).
# - MEDIASERVO_SDK_DIR = the build-c output directory — same freshness
#   guard as cxx (PIT-189 lesson; the per-lane .so trio this band links
#   rides the same build-c pass that produces the client lib the guard
#   checks).
# - Link faces = the bindings lanes' merged per-domain ALIAS
#   (mediaservo::<sdk>, MediaServoBindings.cmake): include roots + lib +
#   rpath — exactly the e2e gcc -I×2 -l triple, registered by the supplier
#   face instead of hand-spelled here. Forward reference of the
#   later-defined ALIAS is the measured cxx pattern (generate-time
#   resolution; the lanes register after this mount).
# - Both debug surfaces through PolyOrch (ruling 4: consumer repos never
#   hand-write what a supplier face generates): polyorch_example_run for
#   the run button, polyorch_native_debug for the F5 row.

cmake_minimum_required(VERSION 3.24)
project(mediaservo_examples_c LANGUAGES C)

get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)

# -- SDK delivery directory (build-c prerequisite; cxx-mirror guard) --
set(MEDIASERVO_SDK_DIR "${REPO_ROOT}/target/debug" CACHE PATH
    "build-c output directory (where libmediaservo_*.so lives)")
if(NOT EXISTS "${MEDIASERVO_SDK_DIR}/libmediaservo_client.so")
    message(FATAL_ERROR
        "no libmediaservo_client.so inside MEDIASERVO_SDK_DIR='${MEDIASERVO_SDK_DIR}' —"
        " run ./mediaservo.sh build bindings before configuring (PIT-189;"
        " the retired build-c alias is gone — build bindings is the .so face)")
endif()

# -- bindings cross-gate (gate-variable form, same R4-2 reasoning as cxx:
#    the lanes register AFTER this mount, so a TARGET-existence test could
#    never pass here; the ALIASes resolve at generate time) --
if(NOT MEDIASERVO_BUILD_BINDINGS)
    message(FATAL_ERROR
        "examples c band requires MEDIASERVO_BUILD_BINDINGS=ON — the link"
        " faces (mediaservo::<sdk>) are registered by the bindings lanes")
endif()

# registry: one directory per example, one .c per directory (the disk<->table
# audit is the registry loop itself — a stale directory FATALs, a missing one
# FATALs; same VE discipline as the rs band's Cargo.toml registry audit)
set(_c_examples deck field link)
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")

foreach(_dir IN LISTS _c_examples)
    if(NOT EXISTS "${CMAKE_CURRENT_LIST_DIR}/${_dir}")
        message(FATAL_ERROR "examples/c registry entry '${_dir}' has no directory on disk")
    endif()
    file(GLOB _src CONFIGURE_DEPENDS "${CMAKE_CURRENT_LIST_DIR}/${_dir}/*.c")
    list(LENGTH _src _nsrc)
    if(NOT _nsrc EQUAL 1)
        message(FATAL_ERROR
            "examples/c/${_dir}/ must hold exactly one .c file (got ${_nsrc})")
    endif()
    add_executable(mediaservo-example-${_dir} "${_src}")
    set_target_properties(mediaservo-example-${_dir} PROPERTIES
        # Per-example folder (2026-11-05 ruling, evolving P8): the build target
        # lives beside its run button in examples/c/<name>/.
        FOLDER "${MEDIASERVO_IDE_ROOT}examples/c/${_dir}")
    target_link_libraries(mediaservo-example-${_dir} PRIVATE mediaservo::${_dir})
    # IDE run button + F5 row (ruling 4): supplier faces only — the COMMAND
    # guard keeps the standalone-band configure (no PolyOrch loaded) green,
    # the same compatibility bonus the cxx band keeps.
    if(COMMAND polyorch_example_run)
        polyorch_example_run(TARGET mediaservo-example-${_dir}
            WORKING_DIRECTORY "${REPO_ROOT}"
            FOLDER "${MEDIASERVO_IDE_ROOT}examples/c/${_dir}")
        add_dependencies(mediaservo-example-${_dir}-run mediaservo-example-${_dir})
    endif()
    if(COMMAND polyorch_native_debug)
        polyorch_native_debug(TARGET mediaservo-example-${_dir}
            SEG "examples/c" CWD "${REPO_ROOT}")
    endif()
endforeach()

message(STATUS "examples c band: deck field link (CMake-ized, ruling 3=B)")
