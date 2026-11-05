# C-D9: dir keeps the plain name, target carries the brand (G4 dir+prefix==target)
add_library(mediaservo-example-imgui_shell STATIC src/shell.cpp src/core.cpp src/texture.cpp)
# IDE group (per-example folder, 2026-11-05 ruling evolving P8): every target
# this directory owns — the lib and core_test below — lives in examples/cxx/imgui_shell/.
set_target_properties(mediaservo-example-imgui_shell PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx/imgui_shell")
target_include_directories(mediaservo-example-imgui_shell PUBLIC ${CMAKE_CURRENT_LIST_DIR}/include)
target_link_libraries(mediaservo-example-imgui_shell PUBLIC mediaservo-imgui)

# X11 real-window form: install a non-exiting Xlib error handler (SDL 3.4.16 has no global handler;
# Xwayland selection/property negotiation None atoms → the default handler exit(1); see
# cmake/gui-deps/patches/sdl3-x11-none-guard.py). The handler works together with the
# events.c NULL guards (missing either means a second-order crash). The probe, the library
# and the include path all come from the gui-deps face (MediaServoGui::X11) -- no
# hand-written find_package(X11) survives here (sr8 P3 seam retirement). Headless profiles
# leave that target undefined → nothing compiles in, zero CI disturbance. The conda libX11
# and SDL dlopen share a soname → a single instance.
if(TARGET MediaServoGui::X11)
    target_compile_definitions(mediaservo-example-imgui_shell PRIVATE MSRTC_X11=1)
    target_link_libraries(mediaservo-example-imgui_shell PUBLIC MediaServoGui::X11)
    message(STATUS "imgui_shell: X11 error handler enabled (gui-deps interface)")
endif()

# viewer_core unit tests (core.cpp has no GUI dependency; compiling separately keeps SDL out of the test binary)
add_executable(mediaservo-core-test tests/core_test.cpp ${CMAKE_CURRENT_LIST_DIR}/src/core.cpp)
set_target_properties(mediaservo-core-test PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx/imgui_shell")  # non-G4: branded by hand; joins its owning dir's folder (2026-11-05)
target_include_directories(mediaservo-core-test PRIVATE ${CMAKE_CURRENT_LIST_DIR}/include)
add_test(NAME core COMMAND mediaservo-core-test)
