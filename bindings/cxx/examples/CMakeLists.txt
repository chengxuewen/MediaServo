# bindings/cxx/examples — GUI/SDK 例子聚合根（p3-gui-viewer W0）
#
# 命令面唯一入口 = `./mediaservo.sh build example [name…]`（G7）；本文件可独立
# `cmake -S . -B build` 配置（兼容白赚=各子目录亦可单独摘走——依赖由本根 prepare）。
#
# 契约（PLAN §3/§6）：
# - 目录名 == target 名 == 产物名（三形合一 G4）：子目录扫描与本文件字面同规则
#   file(GLOB */CMakeLists.txt)（F-A-6），CLI `list example` 复用同一 glob。
# - MEDIASERVO_SDK_DIR = build-c 产物目录（含 libmediaservo_*.so）。缺失/无 .so →
#   FATAL_ERROR 明错指 build-c（PIT-189 部署新鲜度教训的编译面等价物）。
# - GUI 依赖 = 3rdparty/ 本地档案 zip + URL_HASH（D281 oxmgr 同法；台账
#   3rdparty/PROVENANCE-gui-deps.md）。哈希不符 = FetchContent 拒绝展开。

cmake_minimum_required(VERSION 3.24)
project(mediaservo_examples LANGUAGES C CXX)

enable_testing() # 子目录 add_test 生效前提（core 判据面）

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)

# ── SDK 交付目录（build-c 前置）──
set(MEDIASERVO_SDK_DIR "${REPO_ROOT}/target/debug" CACHE PATH
    "build-c 产物目录（libmediaservo_*.so 所在）")
if(NOT EXISTS "${MEDIASERVO_SDK_DIR}/libmediaservo_client.so")
    message(FATAL_ERROR
        "MEDIASERVO_SDK_DIR='${MEDIASERVO_SDK_DIR}' 内无 libmediaservo_client.so —"
        " 先执行 ./mediaservo.sh build-c（或 pixi run build-c）再配置（PIT-189）")
endif()

add_library(ms_client_sdk INTERFACE)
target_include_directories(ms_client_sdk INTERFACE
    "${REPO_ROOT}/bindings/cxx/mediaservo-client-cxx/include"  # client.hpp（C++）
    "${REPO_ROOT}/bindings/cxx/include"                          # detail/result.hpp
    "${REPO_ROOT}/bindings/c/mediaservo-client-c/include"        # client.h（C ABI）
    "${REPO_ROOT}/bindings/c/include")                           # common.h
target_link_libraries(ms_client_sdk INTERFACE
    "${MEDIASERVO_SDK_DIR}/libmediaservo_client.so")

# ── GUI 依赖档案（FetchContent 本地 zip + 哈希）──
include(FetchContent)
FetchContent_Declare(sdl3
    URL "file://${REPO_ROOT}/3rdparty/sdl3-release-3.4.16.zip"
    URL_HASH SHA256=547af2e721e8fc1a60f17acbb01cf6b65a6fa2022d885746b776e2d867adc2b1)
FetchContent_Declare(imgui
    URL "file://${REPO_ROOT}/3rdparty/imgui-v1.92.9b.zip"
    URL_HASH SHA256=008e1f3398e1c6e4d317ae4a05d2dec597bd360f4f3717b1d96e9edb66dba34f)

set(SDL_SHARED OFF CACHE BOOL "" FORCE)      # 例子静态吃 SDL3，产物免运行时库路径钉子
set(SDL_STATIC ON CACHE BOOL "" FORCE)
set(SDL_TESTS OFF CACHE BOOL "" FORCE)
set(SDL_EXAMPLES OFF CACHE BOOL "" FORCE)
# 桌面视频后端：构建机缺 xorg/Wayland dev 头时 SDL 自动探测直接 FATAL（3.4.16 实录
# sdlchecks CheckX11）。W0 档案面 = 官方 headless 组合（docs/README-cmake.md）：
# X11/Wayland 显式 OFF + SDL_UNIX_CONSOLE_BUILD=ON（否则 PrintSummary 无后端 FATAL）。
# 桌面真跑出窗（W3 冒烟 / W5 ubuntu-x11 job）：装 README-linux 包后传
#   -DSDL_X11=ON -DSDL_UNIX_CONSOLE_BUILD=OFF
if(NOT DEFINED SDL_X11)
    set(SDL_X11 OFF CACHE BOOL "" FORCE)
endif()
if(NOT DEFINED SDL_WAYLAND)
    set(SDL_WAYLAND OFF CACHE BOOL "" FORCE)
endif()
# 音频后端（GUI 例子零音频需求）：clean CI（ubuntu-22.04 无 libasound2-dev 等）
# 下 SDL 默认 ON 会 FATAL/find 失败——同 X11 纪律显式 OFF，`if(NOT DEFINED)` 留_OVERRIDE 位。
foreach(_snd ALSA PULSEAUDIO PIPEWIRE SNDIO OSS)
    if(NOT DEFINED SDL_${_snd})
        set(SDL_${_snd} OFF CACHE BOOL "" FORCE)
    endif()
endforeach()

if(NOT DEFINED SDL_UNIX_CONSOLE_BUILD)
    set(SDL_UNIX_CONSOLE_BUILD ON CACHE BOOL "" FORCE)
endif()
FetchContent_MakeAvailable(sdl3)

# imgui 无官方 CMake 构建（上游即源文件集，官方用法=手动编入工程）：
# core + 渲染后端按 docs/BACKENDS.md 清单显式列出。renderer=SDL3 的
# SDL_Renderer 后端（W0 空窗；GPU 纹理面归 W3 再判 sdlgpu3）。
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

# ── 例子聚合：子目录字面同规则扫描（与 CLI list 同源，F-A-6）──
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${REPO_ROOT}/target/examples/bin")

file(GLOB EXAMPLE_DIRS RELATIVE "${CMAKE_CURRENT_LIST_DIR}" CONFIGURE_DEPENDS
     "${CMAKE_CURRENT_LIST_DIR}/*/CMakeLists.txt")
set(_known_targets "")
foreach(entry ${EXAMPLE_DIRS})
    get_filename_component(dir "${entry}" DIRECTORY)
    add_subdirectory("${dir}")
    list(APPEND _known_targets "${dir}")
    # 三形合一门（G4 升格为 configure 期断言，早于任何编译）：目录名必须有同名 target。
    if(NOT TARGET "${dir}")
        message(FATAL_ERROR "例子目录 '${dir}/' 内缺同名 target '${dir}'（三形合一 G4 违约）")
    endif()
endforeach()
message(STATUS "mediaservo examples: ${_known_targets}")
