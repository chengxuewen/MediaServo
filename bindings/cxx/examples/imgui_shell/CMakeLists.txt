add_library(imgui_shell STATIC src/shell.cpp)
target_include_directories(imgui_shell PUBLIC ${CMAKE_CURRENT_LIST_DIR}/include)
target_link_libraries(imgui_shell PUBLIC imgui)
