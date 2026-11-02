add_executable(control_demo main.cpp)
# IDE group (plan ide-folder-hierarchy P8): same examples/ family as the run button's parent
set_target_properties(control_demo PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples")
target_link_libraries(control_demo PRIVATE mediaservo_client_sdk)
