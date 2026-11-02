# mediaservoConfig.cmake — IN-TREE fake (plan bindings-faces P5.5, ruling 9-A;
# VE checked-in precedent cmake/visiaengineConfig.cmake: in-tree and post-install
# consumers read the same mediaservo::<sdk> names — the install-rendered real one
# comes from bindings/c/cmake/mediaservoConfig.cmake.in, SR4/D248).
# Reached via the mediaservo_DIR pin (MediaServoBindings.cmake, VE Ffi:89 shape).
# R4-1: find_package only searches the CONVENTIONAL file name — that is what this
# file is named for. Guard shape mirrors the install template (if(NOT TARGET),
# iron law 7): assert, never redefine.

if(NOT MEDIASERVO_BUILD_BINDINGS)
    message(FATAL_ERROR
        "mediaservo in-tree config: the bindings gate is OFF (MEDIASERVO_BUILD_BINDINGS=0)"
        " — the mediaservo::<sdk> faces do not exist; turn the gate on (or unset"
        " mediaservo_DIR) before find_package(mediaservo).")
endif()

foreach(_sdk field link deck client)
    if(NOT TARGET mediaservo::${_sdk})
        message(FATAL_ERROR
            "mediaservo in-tree config: mediaservo::${_sdk} missing although the bindings"
            " gate is ON — mediaservo_setup_bindings() registers the faces (bindings-faces P2).")
    endif()
endforeach()

set(mediaservo_FOUND TRUE)

# COMPONENTS declarative check (same OpenCV/Boost-pattern text as the install
# template :37-45 — two-state same behavior for the same callsite syntax).
set(mediaservo_KNOWN_COMPONENTS field link deck client)
foreach(_comp IN LISTS mediaservo_FIND_COMPONENTS)
    if(NOT _comp IN_LIST mediaservo_KNOWN_COMPONENTS)
        message(FATAL_ERROR
            "mediaservo: unknown component '${_comp}' (known: ${mediaservo_KNOWN_COMPONENTS})")
    endif()
endforeach()
