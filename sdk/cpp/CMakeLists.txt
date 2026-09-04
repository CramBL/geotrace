cmake_minimum_required(VERSION 3.21)
project(GeoTraceCpp VERSION 0.6.0 LANGUAGES CXX)

# Default to C++17. Callers may override with -DCMAKE_CXX_STANDARD=20.
set(CMAKE_CXX_STANDARD "17" CACHE STRING "C++ standard version")
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# The C++ SDK is header-only: it wraps the C SDK at link time.
find_package(GeoTraceC REQUIRED)

add_library(geotrace_cpp INTERFACE)
add_library(GeoTrace::Cpp ALIAS geotrace_cpp)

target_include_directories(geotrace_cpp INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
)

target_link_libraries(geotrace_cpp INTERFACE GeoTrace::C)
target_compile_features(geotrace_cpp INTERFACE cxx_std_17)

include(GNUInstallDirs)

install(DIRECTORY include/
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}"
)

include(CMakePackageConfigHelpers)

configure_package_config_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/GeoTraceCppConfig.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCppConfig.cmake"
    INSTALL_DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/GeoTraceCpp"
    PATH_VARS CMAKE_INSTALL_LIBDIR CMAKE_INSTALL_INCLUDEDIR
)

write_basic_package_version_file(
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCppConfigVersion.cmake"
    VERSION "${PROJECT_VERSION}"
    COMPATIBILITY SameMajorVersion
)

install(FILES
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCppConfig.cmake"
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCppConfigVersion.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/GeoTraceCpp"
)

option(GEOTRACE_CPP_BUILD_EXAMPLES "Build C++ SDK examples" OFF)
option(BUILD_TESTING "Build C++ SDK tests" OFF)

# Enable testing before the `add_subdirectory` calls so examples can register
# `ctest` smoke tests.
if(GEOTRACE_CPP_BUILD_EXAMPLES OR BUILD_TESTING)
    enable_testing()
endif()

if(GEOTRACE_CPP_BUILD_EXAMPLES)
    add_subdirectory(examples)
endif()

if(BUILD_TESTING)
    add_subdirectory(tests)
endif()
