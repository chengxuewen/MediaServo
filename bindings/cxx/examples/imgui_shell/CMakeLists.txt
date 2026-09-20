add_library(imgui_shell STATIC src/shell.cpp src/core.cpp src/texture.cpp)
target_include_directories(imgui_shell PUBLIC ${CMAKE_CURRENT_LIST_DIR}/include)
target_link_libraries(imgui_shell PUBLIC imgui)

# X11 真窗形态：装非 exit 的 Xlib error handler（SDL 3.4.16 无全局 handler，
# Xwayland selection/property 协商 None 原子 → 默认 handler exit(1)，见
# patches/sdl3-x11-none-guard.py 头注）。handler + events.c NULL 守护配合（缺一即二阶崩）。
# headless（SDL_X11=OFF）不编入，CI 零扰动。conda libX11 与 SDL dlopen 同 soname → 单实例。
if(SDL_X11 AND UNIX AND NOT APPLE)
    find_package(X11 QUIET)
    if(X11_X11_LIB)
        target_compile_definitions(imgui_shell PRIVATE MSRTC_X11=1)
        target_link_libraries(imgui_shell PUBLIC "${X11_X11_LIB}")
        target_include_directories(imgui_shell PRIVATE "${X11_INCLUDE_DIR}")
        message(STATUS "imgui_shell: X11 error handler 启用 (${X11_X11_LIB})")
    endif()
endif()

# viewer_core 单测（core.cpp 无 GUI 依赖，独立编译免拖 SDL 进测试二进制）
add_executable(core_test tests/core_test.cpp ${CMAKE_CURRENT_LIST_DIR}/src/core.cpp)
target_include_directories(core_test PRIVATE ${CMAKE_CURRENT_LIST_DIR}/include)
add_test(NAME core COMMAND core_test)
