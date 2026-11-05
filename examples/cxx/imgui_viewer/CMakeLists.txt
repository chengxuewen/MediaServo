# C-D9: dir keeps the plain name, target carries the brand (G4 dir+prefix==target)
add_executable(mediaservo-example-imgui_viewer
    src/main.cpp
    src/app_model.cpp
    src/app_log.cpp
    src/dock_layout.cpp
    src/sessions.cpp
    src/panels/login.cpp
    src/panels/streams_tree.cpp
    src/panels/stream_info.cpp
    src/panels/log_view.cpp
    src/panels/video_grid.cpp
    src/panels/control.cpp)
set_target_properties(mediaservo-example-imgui_viewer PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx/imgui_viewer")  # per-example folder (2026-11-05, evolving P8; relocation)
target_include_directories(mediaservo-example-imgui_viewer PRIVATE src)
target_link_libraries(mediaservo-example-imgui_viewer PRIVATE mediaservo-example-imgui_shell mediaservo::client mediaservo-examples-toolchain-patch)  # bindings-faces P5 ruling 7-B
