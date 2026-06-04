cmake_minimum_required(VERSION 3.20)
project(GeoTraceC VERSION 0.1.0 LANGUAGES C)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Callers may set GEOTRACE_C_LIB_DIR to a pre-built install prefix.
# If not set, we look alongside the header (useful for in-tree builds).
if(DEFINED GEOTRACE_C_LIB_DIR)
    set(_gtc_lib_search "${GEOTRACE_C_LIB_DIR}/lib" "${GEOTRACE_C_LIB_DIR}")
else()
    set(_gtc_lib_search
        "${CMAKE_CURRENT_SOURCE_DIR}/../../rust/geotrace-c/target/release"
        "${CMAKE_CURRENT_SOURCE_DIR}/../../rust/geotrace-c/target/debug"
    )
endif()

find_library(GEOTRACE_C_LIBRARY
    NAMES geotrace_c
    PATHS ${_gtc_lib_search}
    NO_DEFAULT_PATH
)

if(NOT GEOTRACE_C_LIBRARY)
    message(FATAL_ERROR
        "libgeotrace_c not found. Build it with:\n"
        "  cargo build -p geotrace-c --release\n"
        "or set GEOTRACE_C_LIB_DIR to your install prefix."
    )
endif()

# On Windows we link against the static library (geotrace_c.lib produced by
# Rust's staticlib crate type) to avoid DLL search-path complications.
# On Linux/macOS we use the shared library and embed an RPATH in consumers.
if(WIN32)
    add_library(GeoTrace::C STATIC IMPORTED GLOBAL)
else()
    add_library(GeoTrace::C SHARED IMPORTED GLOBAL)
endif()

set_target_properties(GeoTrace::C PROPERTIES
    IMPORTED_LOCATION "${GEOTRACE_C_LIBRARY}"
    INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_SOURCE_DIR}"
)

include(GNUInstallDirs)

install(FILES geotrace.h
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}"
)

if(WIN32)
    install(FILES "${GEOTRACE_C_LIBRARY}"
        DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    )
else()
    install(IMPORTED_RUNTIME_ARTIFACTS GeoTrace::C
        DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    )
endif()

configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/GeoTraceCConfig.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCConfig.cmake"
    @ONLY
)

install(FILES "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCConfig.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/GeoTraceC"
)

option(GEOTRACE_C_BUILD_EXAMPLES "Build C SDK examples" OFF)

if(GEOTRACE_C_BUILD_EXAMPLES)
    add_subdirectory(examples)
endif()

option(GEOTRACE_C_BUILD_TESTS "Build C SDK tests" OFF)

if(GEOTRACE_C_BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()
