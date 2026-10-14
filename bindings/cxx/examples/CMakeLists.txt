# bindings/cxx/examples — GUI/SDK examples aggregation root (p3-gui-viewer W0)
#
# The single CLI entry point is `./mediaservo.sh build example [name...]` (G7); this file can also be configured standalone with
# `cmake -S . -B build` (a compatibility bonus = each subdirectory can also be lifted out on its own — dependencies are prepared by this root).
#
# Contracts (PLAN §3/§6):
# - Directory name == target name == artifact name (the three-in-one G4): the subdirectory scan follows exactly the same literal rule as this file's
# file(GLOB */CMakeLists.txt) (F-A-6); the CLI `list example` reuses the same glob.
# - MEDIASERVO_SDK_DIR = the build-c output directory (containing libmediaservo_*.so). Missing / no .so →
# FATAL_ERROR with a message pointing at build-c (the compile-surface equivalent of the PIT-189 deployment-freshness lesson).
# - GUI dependencies = local archive zips under 3rdparty/ + URL_HASH (the same method as D281's oxmgr; the ledger is
# 3rdparty/PROVENANCE-gui-deps.md). A hash mismatch = FetchContent refuses to extract.

cmake_minimum_required(VERSION 3.24)
project(mediaservo_examples LANGUAGES C CXX)

enable_testing() # prerequisite for subdirectory add_test to take effect (the core test surface)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)

# -- SDK delivery directory (build-c prerequisite) --
set(MEDIASERVO_SDK_DIR "${REPO_ROOT}/target/debug" CACHE PATH
    "build-c output directory (where libmediaservo_*.so lives)")
if(NOT EXISTS "${MEDIASERVO_SDK_DIR}/libmediaservo_client.so")
    message(FATAL_ERROR
        "no libmediaservo_client.so inside MEDIASERVO_SDK_DIR='${MEDIASERVO_SDK_DIR}' —"
        " run ./mediaservo.sh build-c (or pixi run build-c) before configuring (PIT-189)")
endif()

add_library(mediaservo_client_sdk INTERFACE)
target_include_directories(mediaservo_client_sdk INTERFACE
    "${REPO_ROOT}/bindings/cxx/mediaservo-client-cxx/include"  # client.hpp（C++）
    "${REPO_ROOT}/bindings/cxx/include"                          # detail/result.hpp
    "${REPO_ROOT}/bindings/c/mediaservo-client-c/include"        # client.h（C ABI）
    "${REPO_ROOT}/bindings/c/include")                           # common.h
target_link_libraries(mediaservo_client_sdk INTERFACE
    "${MEDIASERVO_SDK_DIR}/libmediaservo_client.so")
# The SDK is built with the conda toolchain (GLIBCXX_3.4.29/3.4.30 new symbols): when the examples use the system
# compiler chain (the focal header mix fix, 2026-09-28), linking needs rpath-link to see the conda
# libstdc++ to resolve versioned symbols; at runtime the .so's own RPATH backstops it, and the conda compiler also tolerates this flag.
set(_ms_conda_lib "${REPO_ROOT}/.pixi/envs/default/lib")
if(EXISTS "${_ms_conda_lib}/libstdc++.so.6")
    # -L makes the driver-appended -lstdc++ resolve to the conda version (full versioned symbols; first -L wins);
    # rpath-link lets ld resolve the .so's NEEDED chain; rpath keeps the runtime loading the conda libstdc++
    # (system gcc-10 has 3.4.28 without 3.4.29/30 → a runtime version-not-found).
    target_link_options(mediaservo_client_sdk INTERFACE
        "-L${_ms_conda_lib}"
        "-Wl,-rpath-link,${_ms_conda_lib}"
        "-Wl,-rpath,${_ms_conda_lib}")
endif()

# -- GUI dependency archives (FetchContent local zips + hashes) --
include(FetchContent)
FetchContent_Declare(sdl3
    URL "file://${REPO_ROOT}/3rdparty/sdl3-release-3.4.16.zip"
    URL_HASH SHA256=547af2e721e8fc1a60f17acbb01cf6b65a6fa2022d885746b776e2d867adc2b1
    # Upstream bug patch: X11_PickTarget calls XGetAtomName on a None atom from Xwayland's XdndTypeList,
    # → the default Xlib handler exit(1) (unfixed in 3.4.16 / no report / no hint; confirmed 09-20 by comparing main verbatim;
    # four crash sites: PickTarget / SDL_FORMATS×2 / PropertyNotify; the guard mirrors the existing if(name) form at line 1436 in the same file).
    # The archive zip stays byte-for-byte upstream; the guard is applied after extraction (a python script, idempotent per site), harmless headless too.
    PATCH_COMMAND python3 ${CMAKE_CURRENT_LIST_DIR}/patches/sdl3-x11-none-guard.py)
FetchContent_Declare(imgui
    # v1.92.9b-docking = the first-party docking branch's version-pinned tag (commit b48d1af, IMGUI_HAS_DOCK present;
    # imgui.h's IMGUI_VERSION is the same string as master without a suffix — the branch fingerprint is only the IMGUI_HAS_DOCK macro)
    URL "file://${REPO_ROOT}/3rdparty/imgui-v1.92.9b-docking.zip"
    URL_HASH SHA256=0434445157a575f452ff0f2d1681fdd90ea8939e0c6983e6f8e47c51fba1bccd)

set(SDL_SHARED OFF CACHE BOOL "" FORCE)      # the examples link SDL3 statically, sparing the artifacts runtime library-path pins
set(SDL_STATIC ON CACHE BOOL "" FORCE)
set(SDL_TESTS OFF CACHE BOOL "" FORCE)
set(SDL_EXAMPLES OFF CACHE BOOL "" FORCE)
# Desktop video backend: when the build machine lacks xorg/Wayland dev headers, SDL's auto-detection FATALs directly (3.4.16 reality
# in sdlchecks CheckX11). The W0 archive surface = the official headless combination (docs/README-cmake.md):
# X11/Wayland explicitly OFF + SDL_UNIX_CONSOLE_BUILD=ON (otherwise PrintSummary FATALs with no backend).
# Desktop real-window runs (the W3 smoke / W5 ubuntu-x11 job): after installing the README-linux packages pass
#   -DSDL_X11=ON -DSDL_UNIX_CONSOLE_BUILD=OFF
# Desktop real-window automation (the 09-20 touch, the official recipe README L59-60): with the build machine's xorg header package group complete,
# X11 turns on automatically (-DSDL_X11=OFF forces headless); without them → the W0 headless form (zero CI disturbance).
# Three 09-20 pitfalls sidestepped together: ① the conda compiler does not search /usr/include by default (add -isystem); ② pixi's bundled
# x11.pc points at an incomplete conda header directory, shadowing the system one (prepend the system segment to PKG_CONFIG_PATH); ③ SDL has no global
# Xlib error handler (the Xwayland None atom = the upstream bug patch via PATCH_COMMAND).
if(NOT DEFINED SDL_X11)
    set(_msrtc_x11_probe ON)
    foreach(_h "X11/XKBlib.h" "X11/extensions/Xfixes.h" "X11/extensions/XInput2.h"
               "X11/extensions/Xrandr.h" "X11/Xcursor/Xcursor.h")
        find_file(_msrtc_h NAMES ${_h} PATHS /usr/include NO_DEFAULT_PATH)
        if(NOT _msrtc_h)
            set(_msrtc_x11_probe OFF)
        endif()
        unset(_msrtc_h CACHE)
    endforeach()
    if(_msrtc_x11_probe AND CMAKE_SYSTEM_NAME STREQUAL "Linux")
        set(SDL_X11 ON CACHE BOOL "" FORCE)
        # The two 09-20 hacks apply only to the conda compiler: the system gcc's default chain already includes /usr/include,
        # and another -isystem triggers include_next dedup breakage (c++/10/cstdlib cannot find stdlib.h,
        # the focal incident of 2026-09-28). The conda side is unchanged (zero disturbance for CI / old Jetson).
        if(CMAKE_C_COMPILER MATCHES "conda|envs/default")
            string(PREPEND CMAKE_C_FLAGS "-isystem /usr/include ")
            string(PREPEND CMAKE_CXX_FLAGS "-isystem /usr/include ")
            set(ENV{PKG_CONFIG_PATH} "/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig:$ENV{PKG_CONFIG_PATH}")
        endif()
        message(STATUS "GUI examples: detected the system xorg header package group → SDL_X11=ON (windowed form; -DSDL_X11=OFF forces headless)")
    else()
        set(SDL_X11 OFF CACHE BOOL "" FORCE)
    endif()
endif()
if(NOT DEFINED SDL_WAYLAND)
    set(SDL_WAYLAND OFF CACHE BOOL "" FORCE)
endif()
# Audio backend (the GUI examples need no audio): on a clean CI (ubuntu-22.04 without libasound2-dev etc.),
# SDL's default ON FATALs/find fails — explicitly OFF with the same X11 discipline; `if(NOT DEFINED)` keeps an _OVERRIDE slot.
foreach(_snd ALSA PULSEAUDIO PIPEWIRE SNDIO OSS)
    if(NOT DEFINED SDL_${_snd})
        set(SDL_${_snd} OFF CACHE BOOL "" FORCE)
    endif()
endforeach()

if(NOT DEFINED SDL_UNIX_CONSOLE_BUILD)
    if(SDL_X11)
        set(SDL_UNIX_CONSOLE_BUILD OFF CACHE BOOL "" FORCE)  # the official real-window recipe (console only needed with no backend)
    else()
        set(SDL_UNIX_CONSOLE_BUILD ON CACHE BOOL "" FORCE)
    endif()
endif()
FetchContent_MakeAvailable(sdl3)

# imgui has no official CMake build (upstream is just a set of source files; the official usage = compile them into your project manually):
# core + the rendering backend are listed explicitly per the docs/BACKENDS.md inventory. For renderer=SDL3 the
# SDL_Renderer backend (the W0 empty window; the GPU texture surface goes to W3, which will judge sdlgpu3).
FetchContent_Populate(imgui)
add_library(imgui STATIC
    "${imgui_SOURCE_DIR}/imgui.cpp"
    "${imgui_SOURCE_DIR}/imgui_draw.cpp"
    "${imgui_SOURCE_DIR}/imgui_tables.cpp"
    "${imgui_SOURCE_DIR}/imgui_widgets.cpp"
    "${imgui_SOURCE_DIR}/backends/imgui_impl_sdl3.cpp"
    "${imgui_SOURCE_DIR}/backends/imgui_impl_sdlrenderer3.cpp")
target_include_directories(imgui PUBLIC "${imgui_SOURCE_DIR}" "${imgui_SOURCE_DIR}/backends")
target_link_libraries(imgui PUBLIC SDL3::SDL3)

# -- Example aggregation: subdirectory scan with the same literal rule (same source as the CLI list, F-A-6) --
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")

file(GLOB EXAMPLE_DIRS RELATIVE "${CMAKE_CURRENT_LIST_DIR}" CONFIGURE_DEPENDS
     "${CMAKE_CURRENT_LIST_DIR}/*/CMakeLists.txt")
set(_known_targets "")
foreach(entry ${EXAMPLE_DIRS})
    get_filename_component(dir "${entry}" DIRECTORY)
    add_subdirectory("${dir}")
    list(APPEND _known_targets "${dir}")
    # The three-in-one gate (G4 promoted to a configure-time assertion, before any compilation): a directory must have a same-named target.
    if(NOT TARGET "${dir}")
        message(FATAL_ERROR "example directory '${dir}/' lacks a same-named target '${dir}' (a G4 three-in-one violation)")
    endif()
endforeach()
message(STATUS "mediaservo examples: ${_known_targets}")
