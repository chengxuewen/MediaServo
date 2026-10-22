# MediaServoPolyOrch.cmake — PolyOrch embed shim (D39 sentinel contract)
#
# Semantics (mirrors the VisiaEngine shim, the measured sibling precedent):
#   * Sentinel present in this configure (a superbuild embedding MediaServo already
#     ran add_subdirectory(PolyOrch)): REUSE it. First-wins (PolyOrch D39): the
#     winning copy's code is live everywhere; this shim only reports which copy won.
#   * No sentinel: self-embed our pinned submodule copy at the repo root.
#     Version-conflict policy (ModuKit D67 grammar): the pinned gitlink SHA is a
#     "lowest tested version" declaration, NOT an enforcement — the effective copy
#     is whatever won first-wins. No version-range negotiation until PolyOrch
#     ships 1.0 + semver.
#   * API floor (two forms, per the D39-era measured discipline): keyword-level
#     capability is invisible to COMMAND probes, so it rides the sentinel's
#     INTERFACE_POLYORCH_VERSION; a bare (pre-D39) sentinel fails the property
#     read — same loud discipline upstream. Floor = 0.5.0 (D44 rust clean
#     PACKAGE form, superseding the D42 LIB floor; our examples/crates lanes from S2
#     onward need it).

# Repo anchor: this file lives in <repo>/cmake/, so defining-file dir + ".." is the
# repo root in BOTH standalone and superbuild (ms_rtc shell / third-party embed) mode —
# CMAKE_SOURCE_DIR is the WRONG anchor under a superbuild (it points at the outer root).
# All MediaServo modules consume MEDIASERVO_REPO_ROOT (F24).
get_filename_component(MEDIASERVO_REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
set(MEDIASERVO_REPO_ROOT "${MEDIASERVO_REPO_ROOT}" CACHE INTERNAL "MediaServo repo root (superbuild-safe anchor)")
# Escape hatch: a superbuild may point at its own checkout via
#   -DMEDIASERVO_POLYORCH_DIR=<path>.

function(mediaservo_ensure_polyorch)
    if(TARGET PolyOrch::PolyOrch)
        get_target_property(_po_dir PolyOrch::PolyOrch INTERFACE_POLYORCH_CMAKE_DIR)
        get_target_property(_po_ver PolyOrch::PolyOrch INTERFACE_POLYORCH_VERSION)
        if(NOT _po_dir OR NOT _po_ver)
            message(FATAL_ERROR
                "MediaServo: PolyOrch sentinel lacks INTERFACE_POLYORCH_CMAKE_DIR/VERSION "
                "(pre-D39 bare sentinel). Fix: upgrade the winning outer PolyOrch checkout "
                "to a D39-complete version (>= 0.5.0).")
        endif()
        if(_po_ver VERSION_LESS "0.5.0")
            message(FATAL_ERROR
                "MediaServo needs PolyOrch >= 0.5.0 (rust clean PACKAGE form, D44) but the winning "
                "sentinel reports '${_po_ver}' — in reuse mode the outer copy is too old; "
                "upgrade the superbuild's PolyOrch checkout or unshadow this repo's pinned copy.")
        endif()
        message(STATUS
            "MediaServo: reusing PolyOrch ${_po_ver} at ${_po_dir} "
            "(first-wins: registered by an outer scope)")
        return()
    endif()

    if(MEDIASERVO_POLYORCH_DIR)
        set(_po "${MEDIASERVO_POLYORCH_DIR}")
    else()
        # Repo-relative (superbuild-safe, F24): the pinned submodule checkout
        # always lives next to this module's repo root, wherever the outer
        # project happens to be.
        set(_po "${MEDIASERVO_REPO_ROOT}/PolyOrch")
    endif()
    if(NOT EXISTS "${_po}/CMakeLists.txt")
        message(FATAL_ERROR
            "MediaServo: no PolyOrch::PolyOrch sentinel in this configure, and no checkout "
            "at '${_po}'. Fix: run 'git submodule update --init --recursive' (pins the "
            "tested SHA), or set -DMEDIASERVO_POLYORCH_DIR=<path> to supply a different "
            "PolyOrch checkout.")
    endif()
    message(STATUS "MediaServo: embedding pinned PolyOrch at ${_po}")
    add_subdirectory("${_po}" "${CMAKE_BINARY_DIR}/_polyorch" EXCLUDE_FROM_ALL)

    # post-embed floor check (self-embed path): D39 properties must be delivered
    get_target_property(_po_ver PolyOrch::PolyOrch INTERFACE_POLYORCH_VERSION)
    if(NOT _po_ver OR _po_ver VERSION_LESS "0.5.0")
        message(FATAL_ERROR
            "MediaServo: the pinned PolyOrch checkout reports '${_po_ver}', below the "
            "required floor 0.5.0 (rust clean PACKAGE form, D44). Bump the gitlink "
            "to a D39/D44-complete SHA.")
    endif()
endfunction()
