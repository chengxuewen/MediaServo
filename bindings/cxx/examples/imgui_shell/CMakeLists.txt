add_library(imgui_shell STATIC src/shell.cpp src/core.cpp src/texture.cpp)
target_include_directories(imgui_shell PUBLIC ${CMAKE_CURRENT_LIST_DIR}/include)
target_link_libraries(imgui_shell PUBLIC imgui)

# X11 real-window form: install a non-exiting Xlib error handler (SDL 3.4.16 has no global handler;
# Xwayland selection/property negotiation None atoms → the default handler exit(1); see
# the patches/sdl3-x11-none-guard.py header note). The handler works together with the events.c NULL guards (missing either means a second-order crash).
# Not compiled in for headless (SDL_X11=OFF), zero CI disturbance. The conda libX11 and SDL dlopen share a soname → a single instance.
if(SDL_X11 AND UNIX AND NOT APPLE)
    find_package(X11 QUIET)
    if(X11_X11_LIB)
        target_compile_definitions(imgui_shell PRIVATE MSRTC_X11=1)
        target_link_libraries(imgui_shell PUBLIC "${X11_X11_LIB}")
        target_include_directories(imgui_shell PRIVATE "${X11_INCLUDE_DIR}")
        message(STATUS "imgui_shell: X11 error handler enabled (${X11_X11_LIB})")
    endif()
endif()

# viewer_core unit tests (core.cpp has no GUI dependency; compiling separately keeps SDL out of the test binary)
add_executable(core_test tests/core_test.cpp ${CMAKE_CURRENT_LIST_DIR}/src/core.cpp)
target_include_directories(core_test PRIVATE ${CMAKE_CURRENT_LIST_DIR}/include)
add_test(NAME core COMMAND core_test)
