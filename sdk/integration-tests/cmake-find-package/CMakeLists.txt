cmake_minimum_required(VERSION 3.21)
project(GeoTraceFindPackageSmoke C CXX)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(GeoTraceC REQUIRED)
find_package(GeoTraceCpp REQUIRED)

add_executable(smoke_c smoke_c.c)
target_link_libraries(smoke_c PRIVATE GeoTrace::C)

add_executable(smoke_cpp smoke_cpp.cpp)
target_link_libraries(smoke_cpp PRIVATE GeoTrace::Cpp)

foreach(smoke_target smoke_c smoke_cpp)
    # Neither program depends on the directory it starts from: each writes and
    # reads its .gtd file at this absolute path.
    target_compile_definitions(${smoke_target} PRIVATE
        GEOTRACE_SMOKE_PATH="${CMAKE_CURRENT_BINARY_DIR}/${smoke_target}.gtd"
    )
    # Embed an RPATH so each program finds the shared library without
    # LD_LIBRARY_PATH / DYLD_LIBRARY_PATH. For a static GeoTrace::C the entry is
    # harmless.
    set_target_properties(${smoke_target} PROPERTIES
        BUILD_RPATH "$<TARGET_FILE_DIR:GeoTrace::C>"
    )
endforeach()
