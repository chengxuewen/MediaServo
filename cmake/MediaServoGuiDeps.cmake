#-----------------------------------------------------------------------------------------------------------------------
# MediaServoGuiDeps.cmake -- the GUI dependency face (sr8 P3; D50, F7/F8/F9/F10).
#
# Policy here, mechanism in PolyOrch. cmake/gui-deps/ is the SINGLE definition
# point (r4 H1): no wrapper, recipe or patch may live under examples/ -- that
# is what makes the examples-side closure gate able to scan its own band.
#   * SDL3  = independent build/install through polyorch_thirdparty_recipe.
#             Static facts (version/url/hashes/patch command/option tables)
#             live in cmake/gui-deps/recipes/sdl3.json. The X11 header probe
#             (dynamic decision) stays HERE -- policy never enters the JSON.
#             Consumed through the upstream SDL3Config.cmake as
#             SDL3::SDL3-static; consumers get the alias MediaServoGui::SDL3
#             (F7: upstream target names are not ours to rename).
#   * ImGui = source wrapper (F8): the same loader in fetch_only mode acquires
#             and verifies the archive; mediaservo-imgui compiles the six TUs
#             with the CONSUMER toolchain (C++ ABI follows the consumer; no
#             independent build/install/package-cache entry).
#   * configure NEVER builds dependencies (H2): this file only imports and
#             wires targets. A missing SDL install prefix is a FATAL naming
#             the explicit verb -- an IDE reconfigure must never trigger
#             minutes of compilation.
#   * Build anchor (F10 dual-track): the anchor is pinned at this call site
#             (repo-root derived, overridable by the plain variable
#             MEDIASERVO_GUI_DEPS_ANCHOR) and NEVER read from the environment.
#             The shell verb resolves MSRTC_BUILD_ROOT on its own side; both
#             default to <repo>/build/3rdparty.
#
# Explicit verb: ./mediaservo.sh build gui-deps [--profile auto|headless|linux-desktop] [--release]
#-----------------------------------------------------------------------------------------------------------------------

# This file's directory, captured at INCLUDE time: inside the macro below
# CMAKE_CURRENT_LIST_DIR is the CALLER's directory (macro scope), a measured
# trap that would send the recipe lookup under examples/cxx/.
get_filename_component(MEDIASERVO_GUI_DEPS_DIR "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
set(MEDIASERVO_GUI_DEPS_DIR "${MEDIASERVO_GUI_DEPS_DIR}" CACHE INTERNAL "MediaServo gui-deps module directory (recipes/patches live beside it)")

# mediaservo_gui_deps_profile(<out>) -- the dynamic decision shared with the
# shell verb's --profile auto rule (keep both header lists aligned; the shell
# copy is _ms_x11_headers_present in mediaservo.sh).
function(mediaservo_gui_deps_profile out)
    set(_prof "headless")
    if(UNIX AND NOT APPLE AND CMAKE_SYSTEM_NAME STREQUAL "Linux")
        set(_probe ON)
        foreach(_h "X11/Xlib.h" "X11/XKBlib.h" "X11/extensions/Xfixes.h" "X11/extensions/XInput2.h"
                   "X11/extensions/Xrandr.h" "X11/Xcursor/Xcursor.h" "X11/extensions/Xext.h")
            find_file(_ms_gd_h NAMES "${_h}" PATHS /usr/include NO_DEFAULT_PATH)
            if(NOT _ms_gd_h)
                set(_probe OFF)
            endif()
            unset(_ms_gd_h CACHE)
        endforeach()
        if(_probe)
            set(_prof "linux-desktop")
        endif()
    endif()
    set(${out} "${_prof}" PARENT_SCOPE)
endfunction()

# mediaservo_gui_deps() -- import SDL3 + define the ImGui wrapper.
# A MACRO on purpose, same discipline as polyorch_thirdparty_import: the
# upstream config file defines imported targets and companion variables in
# the CALLER's scope; a function boundary would strand them. Internals carry
# _ms_gd_ (macros share the caller scope).
macro(mediaservo_gui_deps)
    if(NOT COMMAND polyorch_thirdparty_recipe OR NOT COMMAND polyorch_thirdparty_import)
        message(FATAL_ERROR "mediaservo_gui_deps: the PolyOrch third-party face is not loaded -- include PolyOrchThirdPartyHelpers.cmake (mediaservo_ensure_polyorch wires it) first")
    endif()
    if(NOT MEDIASERVO_REPO_ROOT)
        message(FATAL_ERROR "mediaservo_gui_deps: MEDIASERVO_REPO_ROOT is not set (include cmake/MediaServoPolyOrch.cmake first)")
    endif()
    if(CMAKE_CONFIGURATION_TYPES)
        message(FATAL_ERROR "mediaservo_gui_deps: multi-config generators are not supported by this anchor yet (O-3 seam: per-config install prefixes land with the IDE face); use a single-config generator")
    endif()
    if(NOT CMAKE_BUILD_TYPE)
        message(FATAL_ERROR "mediaservo_gui_deps: CMAKE_BUILD_TYPE is empty -- the SDL install anchor is per effective config (O-3: silence here would collapse configs); pass -DCMAKE_BUILD_TYPE=Debug|Release")
    endif()
    # Explicit profile override (the r4 L4 escape hatch): a dev box with X11
    # headers can force the headless form (and vice versa) without faking a
    # missing header set -- pair it with `build gui-deps --profile <same>`.
    if(DEFINED MEDIASERVO_GUI_DEPS_PROFILE AND NOT "${MEDIASERVO_GUI_DEPS_PROFILE}" STREQUAL "")
        set(_ms_gd_profile "${MEDIASERVO_GUI_DEPS_PROFILE}")
        if(NOT _ms_gd_profile STREQUAL "headless" AND NOT _ms_gd_profile STREQUAL "linux-desktop")
            message(FATAL_ERROR "mediaservo_gui_deps: MEDIASERVO_GUI_DEPS_PROFILE='${_ms_gd_profile}' is not a known profile (headless|linux-desktop)")
        endif()
    else()
        mediaservo_gui_deps_profile(_ms_gd_profile)
    endif()
    if(NOT DEFINED MEDIASERVO_GUI_DEPS_ANCHOR)
        set(MEDIASERVO_GUI_DEPS_ANCHOR "${MEDIASERVO_REPO_ROOT}/build/3rdparty" CACHE PATH
            "GUI dependency build anchor (F10 callsite pin; the shell verb resolves MSRTC_BUILD_ROOT on its side)")
    endif()
    # ---- provider path tag (P4) ----
    # The install tree an import reads must be the tree the VERB built under
    # the SAME provider: provider identity rides the directory name (the cache
    # key already carries it -- the paths must not alias either). Default none
    # keeps the P3 layout byte-for-byte: <anchor>/<profile>/... (F3=A: pixi is
    # implemented but never the default).
    set(_ms_gd_provider "none")
    if(DEFINED MEDIASERVO_GUI_DEPS_PROVIDER AND NOT "${MEDIASERVO_GUI_DEPS_PROVIDER}" STREQUAL "")
        set(_ms_gd_provider "${MEDIASERVO_GUI_DEPS_PROVIDER}")
    endif()
    if(NOT _ms_gd_provider MATCHES "^(none|system|pixi)$")
        message(FATAL_ERROR "mediaservo_gui_deps: MEDIASERVO_GUI_DEPS_PROVIDER='${_ms_gd_provider}' is not a known provider (none|system|pixi)")
    endif()
    set(_ms_gd_ptag "")
    if(NOT _ms_gd_provider STREQUAL "none")
        set(_ms_gd_ptag "-${_ms_gd_provider}")
    endif()
    set(_ms_gd_work "${MEDIASERVO_GUI_DEPS_ANCHOR}/${_ms_gd_profile}${_ms_gd_ptag}")
    set(_ms_gd_sdl_install "${_ms_gd_work}/sdl3/install-${CMAKE_BUILD_TYPE}")
    if(NOT EXISTS "${_ms_gd_sdl_install}/lib/cmake/SDL3/SDL3Config.cmake")
        set(_ms_gd_verb "--profile ${_ms_gd_profile}")
        if(NOT _ms_gd_provider STREQUAL "none")
            set(_ms_gd_verb_prov " --provider ${_ms_gd_provider}")
        else()
            set(_ms_gd_verb_prov "")
        endif()
        if(CMAKE_BUILD_TYPE STREQUAL "Release")
            string(APPEND _ms_gd_verb " --release")
        endif()
        message(FATAL_ERROR
            "mediaservo_gui_deps: no SDL3 install prefix at '${_ms_gd_sdl_install}' (profile=${_ms_gd_profile}, provider=${_ms_gd_provider}, config=${CMAKE_BUILD_TYPE}).\n"
            "Configure never builds dependencies (H2). Run the explicit verb first:\n"
            "  ./mediaservo.sh build gui-deps ${_ms_gd_verb}${_ms_gd_verb_prov}\n"
            "If the shell resolved MSRTC_BUILD_ROOT, mirror it here: -DMEDIASERVO_GUI_DEPS_ANCHOR=<MSRTC_BUILD_ROOT>/3rdparty")
    endif()
    polyorch_thirdparty_import(NAME sdl3 INSTALL_DIR "${_ms_gd_sdl_install}"
        FIND_PACKAGE_NAME SDL3 COMPONENTS SDL3-static
        PROVIDED_TARGETS SDL3::SDL3-static ALIAS MediaServoGui::SDL3)

    # X11 (linux-desktop only): the app-side Xlib error handler needs the
    # system libX11 + headers. One named interface target replaces every
    # hand-written find_package(X11) under examples/ (P3 seam retirement).
    if(_ms_gd_profile STREQUAL "linux-desktop" AND UNIX AND NOT APPLE AND NOT TARGET MediaServoGui::X11)
        find_package(X11 QUIET)
        if(X11_X11_LIB)
            add_library(MediaServoGui::X11 INTERFACE IMPORTED)
            set_target_properties(MediaServoGui::X11 PROPERTIES
                INTERFACE_LINK_LIBRARIES "${X11_X11_LIB}"
                INTERFACE_INCLUDE_DIRECTORIES "${X11_INCLUDE_DIR}")
            message(STATUS "gui-deps: MediaServoGui::X11 -> ${X11_X11_LIB}")
        endif()
    endif()

    # ImGui source wrapper (F8): fetch-only recipe, compiled by the consumer.
    if(TARGET mediaservo-imgui)
        message(FATAL_ERROR "mediaservo_gui_deps: mediaservo-imgui already exists -- the wrapper has exactly ONE definition point (this file; r4 H1)")
    endif()
    polyorch_thirdparty_recipe(RECIPE "${MEDIASERVO_GUI_DEPS_DIR}/gui-deps/recipes/imgui.json"
        WORK_DIR "${MEDIASERVO_GUI_DEPS_ANCHOR}" BUILD_TYPE "${CMAKE_BUILD_TYPE}"
        SOURCE_DIR_OUT _ms_gd_imgui_src)
    add_library(mediaservo-imgui STATIC
        "${_ms_gd_imgui_src}/imgui.cpp"
        "${_ms_gd_imgui_src}/imgui_draw.cpp"
        "${_ms_gd_imgui_src}/imgui_tables.cpp"
        "${_ms_gd_imgui_src}/imgui_widgets.cpp"
        "${_ms_gd_imgui_src}/backends/imgui_impl_sdl3.cpp"
        "${_ms_gd_imgui_src}/backends/imgui_impl_sdlrenderer3.cpp")
    if(DEFINED MEDIASERVO_IDE_ROOT AND NOT "${MEDIASERVO_IDE_ROOT}" STREQUAL "")
        set_target_properties(mediaservo-imgui PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}gui-deps")
    endif()
    target_include_directories(mediaservo-imgui PUBLIC "${_ms_gd_imgui_src}" "${_ms_gd_imgui_src}/backends")
    target_link_libraries(mediaservo-imgui PUBLIC MediaServoGui::SDL3)
    message(STATUS "gui-deps: profile=${_ms_gd_profile} config=${CMAKE_BUILD_TYPE} SDL3=${_ms_gd_sdl_install} imgui=${_ms_gd_imgui_src}")
endmacro()
