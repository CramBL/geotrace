cmake_minimum_required(VERSION 3.21)
project(GeoTraceCpp VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# The C++ SDK is header-only — it wraps the C SDK at link time.
find_package(GeoTraceC REQUIRED)

add_library(geotrace_cpp INTERFACE)
add_library(GeoTrace::Cpp ALIAS geotrace_cpp)

target_include_directories(geotrace_cpp INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
)

target_link_libraries(geotrace_cpp INTERFACE GeoTrace::C)
target_compile_features(geotrace_cpp INTERFACE cxx_std_17)

# ── Install ───────────────────────────────────────────────────────────────────

include(GNUInstallDirs)

install(DIRECTORY include/
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}"
)

configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/GeoTraceCppConfig.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCppConfig.cmake"
    @ONLY
)

install(FILES "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCppConfig.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/GeoTraceCpp"
)

# ── Examples ─────────────────────────────────────────────────────────────────

option(GEOTRACE_CPP_BUILD_EXAMPLES "Build C++ SDK examples" OFF)

if(GEOTRACE_CPP_BUILD_EXAMPLES)
    add_subdirectory(examples)
endif()

# ── Tests ─────────────────────────────────────────────────────────────────────

option(BUILD_TESTING "Build C++ SDK tests" OFF)

if(BUILD_TESTING)
    enable_testing()
    add_subdirectory(tests)
endif()
