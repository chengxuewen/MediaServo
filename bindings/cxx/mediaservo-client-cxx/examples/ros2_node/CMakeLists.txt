cmake_minimum_required(VERSION 3.16)
project(mediaservo_control_relay LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(ament_cmake REQUIRED)
find_package(rclcpp REQUIRED)
find_package(std_msgs REQUIRED)
# sdk-client 安装面（libmediaservo_client + mediaservo/client.hpp + .pc）
find_package(PkgConfig REQUIRED)
pkg_check_modules(MEDIASERVO_CLIENT REQUIRED mediaservo-client)

add_executable(control_relay_node src/control_relay_node.cpp)
target_include_directories(control_relay_node PRIVATE ${MEDIASERVO_CLIENT_INCLUDE_DIRS})
target_link_directories(control_relay_node PRIVATE ${MEDIASERVO_CLIENT_LIBRARY_DIRS})
target_link_libraries(control_relay_node ${MEDIASERVO_CLIENT_LIBRARIES})
ament_target_dependencies(control_relay_node rclcpp std_msgs)

install(TARGETS control_relay_node DESTINATION lib/${PROJECT_NAME})
ament_package()
