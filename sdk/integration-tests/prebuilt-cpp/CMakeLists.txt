cmake_minimum_required(VERSION 3.21)
project(GeoTraceCppSmoke CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# GeoTraceCpp's package config pulls in GeoTraceC (and the GeoTrace::C target)
# via find_dependency, so consumers only need to find the C++ package.
find_package(GeoTraceCpp REQUIRED)

add_executable(smoke_cpp smoke_cpp.cpp)
target_link_libraries(smoke_cpp PRIVATE GeoTrace::Cpp)

# Embed an RPATH so the binary finds a shared GeoTrace::C without
# LD_LIBRARY_PATH / DYLD_LIBRARY_PATH (harmless for a static library).
set_target_properties(smoke_cpp PROPERTIES
    BUILD_RPATH "$<TARGET_FILE_DIR:GeoTrace::C>"
)

include(CTest)
add_test(NAME smoke_cpp COMMAND smoke_cpp)
