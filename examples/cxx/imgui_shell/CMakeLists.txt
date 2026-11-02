# C-D9: dir keeps the plain name, target carries the brand (G4 dir+prefix==target)
add_library(mediaservo-example-imgui_shell STATIC src/shell.cpp src/core.cpp src/texture.cpp)
# IDE group (plan ide-folder-hierarchy P8, R4-1): core_test below is NOT a same-named
# dir target — the band root loop can never carry it; both join by definition point.
set_target_properties(mediaservo-example-imgui_shell PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx")
target_include_directories(mediaservo-example-imgui_shell PUBLIC ${CMAKE_CURRENT_LIST_DIR}/include)
target_link_libraries(mediaservo-example-imgui_shell PUBLIC mediaservo-imgui)

# X11 real-window form: install a non-exiting Xlib error handler (SDL 3.4.16 has no global handler;
# Xwayland selection/property negotiation None atoms → the default handler exit(1); see
# the patches/sdl3-x11-none-guard.py header note). The handler works together with the events.c NULL guards (missing either means a second-order crash).
# Not compiled in for headless (SDL_X11=OFF), zero CI disturbance. The conda libX11 and SDL dlopen share a soname → a single instance.
if(SDL_X11 AND UNIX AND NOT APPLE)
    find_package(X11 QUIET)
    if(X11_X11_LIB)
        target_compile_definitions(mediaservo-example-imgui_shell PRIVATE MSRTC_X11=1)
        target_link_libraries(mediaservo-example-imgui_shell PUBLIC "${X11_X11_LIB}")
        target_include_directories(mediaservo-example-imgui_shell PRIVATE "${X11_INCLUDE_DIR}")
        message(STATUS "imgui_shell: X11 error handler enabled (${X11_X11_LIB})")
    endif()
endif()

# viewer_core unit tests (core.cpp has no GUI dependency; compiling separately keeps SDL out of the test binary)
add_executable(mediaservo-core-test tests/core_test.cpp ${CMAKE_CURRENT_LIST_DIR}/src/core.cpp)
set_target_properties(mediaservo-core-test PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx")  # plan P8 (non-G4: branded by hand, no dir partner)
target_include_directories(mediaservo-core-test PRIVATE ${CMAKE_CURRENT_LIST_DIR}/include)
add_test(NAME core COMMAND mediaservo-core-test)
