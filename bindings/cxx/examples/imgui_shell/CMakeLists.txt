add_library(imgui_shell STATIC src/shell.cpp src/core.cpp src/texture.cpp)
target_include_directories(imgui_shell PUBLIC ${CMAKE_CURRENT_LIST_DIR}/include)
target_link_libraries(imgui_shell PUBLIC imgui)

# viewer_core 单测（core.cpp 无 GUI 依赖，独立编译免拖 SDL 进测试二进制）
add_executable(core_test tests/core_test.cpp ${CMAKE_CURRENT_LIST_DIR}/src/core.cpp)
target_include_directories(core_test PRIVATE ${CMAKE_CURRENT_LIST_DIR}/include)
add_test(NAME core COMMAND core_test)
