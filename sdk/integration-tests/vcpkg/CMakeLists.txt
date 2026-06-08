cmake_minimum_required(VERSION 3.21)
project(GeoTraceCVcpkgSmoke C)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_STANDARD_REQUIRED ON)

find_package(GeoTraceC REQUIRED)

add_executable(smoke_c smoke_c.c)
target_link_libraries(smoke_c PRIVATE GeoTrace::C)
