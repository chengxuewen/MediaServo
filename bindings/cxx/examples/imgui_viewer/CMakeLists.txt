add_executable(imgui_viewer src/main.cpp)
target_link_libraries(imgui_viewer PRIVATE imgui_shell ms_client_sdk)
