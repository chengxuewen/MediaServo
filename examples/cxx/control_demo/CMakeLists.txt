# C-D9: dir keeps the plain name, target carries the brand (G4 dir+prefix==target)
add_executable(mediaservo-example-control_demo main.cpp)
# IDE group (plan ide-folder-hierarchy P8 + this round's relocation): definition-point sets
set_target_properties(mediaservo-example-control_demo PROPERTIES FOLDER "${MEDIASERVO_IDE_ROOT}examples/cxx/control_demo")  # per-example folder (2026-11-05, evolving P8)
target_link_libraries(mediaservo-example-control_demo PRIVATE mediaservo::client mediaservo-examples-toolchain-patch)  # bindings-faces P5 ruling 7-B: direct link-face consumption
