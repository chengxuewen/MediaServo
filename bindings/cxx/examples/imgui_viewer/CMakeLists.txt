add_executable(imgui_viewer
    src/main.cpp
    src/app_model.cpp
    src/sessions.cpp
    src/panels/login.cpp
    src/panels/rooms.cpp
    src/panels/video_grid.cpp
    src/panels/control.cpp)
target_include_directories(imgui_viewer PRIVATE src)
target_link_libraries(imgui_viewer PRIVATE imgui_shell mediaservo_client_sdk)
