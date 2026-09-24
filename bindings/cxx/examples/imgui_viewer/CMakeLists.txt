add_executable(imgui_viewer
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
target_include_directories(imgui_viewer PRIVATE src)
target_link_libraries(imgui_viewer PRIVATE imgui_shell mediaservo_client_sdk)
