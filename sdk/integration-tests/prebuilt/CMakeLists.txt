cmake_minimum_required(VERSION 3.21)
project(GeoTraceCSmoke C)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_STANDARD_REQUIRED ON)

find_package(GeoTraceC REQUIRED)

add_executable(smoke_c smoke_c.c)
target_link_libraries(smoke_c PRIVATE GeoTrace::C)

# Embed an RPATH so `ctest` finds the shared library without LD_LIBRARY_PATH /
# DYLD_LIBRARY_PATH.  For a static GeoTrace::C the RPATH entry is harmless
# (no DLL to locate at runtime).
set_target_properties(smoke_c PROPERTIES
    BUILD_RPATH "$<TARGET_FILE_DIR:GeoTrace::C>"
)

include(CTest)
add_test(NAME smoke_c COMMAND smoke_c)
