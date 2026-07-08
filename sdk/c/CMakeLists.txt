cmake_minimum_required(VERSION 3.21)
project(GeoTraceC VERSION 0.4.0 LANGUAGES C)

set(CMAKE_C_STANDARD 99)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Enable static linking on all platforms, not just Windows.
# Windows always uses static to avoid DLL search-path complications;
# Linux/macOS default to shared but callers can opt in to static.
option(GEOTRACE_C_STATIC "Link against the static library (.a / .lib)" OFF)

# Callers may set GEOTRACE_C_LIB_DIR to a pre-built install prefix.
# If not set, we look alongside the header (useful for in-tree builds).
if(DEFINED GEOTRACE_C_LIB_DIR)
    set(_gtc_lib_search "${GEOTRACE_C_LIB_DIR}/lib" "${GEOTRACE_C_LIB_DIR}")
else()
    set(_gtc_lib_search
        "${CMAKE_CURRENT_SOURCE_DIR}/../../target/release"
        "${CMAKE_CURRENT_SOURCE_DIR}/../../target/debug"
    )
endif()

# Force static (.a / .lib) lookup when the option is set so that
# find_library doesn't inadvertently pick up a shared library if both exist.
if(GEOTRACE_C_STATIC OR WIN32)
    set(_gtc_saved_suffixes "${CMAKE_FIND_LIBRARY_SUFFIXES}")
    if(MSVC)
        set(CMAKE_FIND_LIBRARY_SUFFIXES ".lib")
    else()
        set(CMAKE_FIND_LIBRARY_SUFFIXES ".a")
    endif()
endif()

find_library(GEOTRACE_C_LIBRARY
    NAMES geotrace_c
    PATHS ${_gtc_lib_search}
    NO_DEFAULT_PATH
)

if(GEOTRACE_C_STATIC OR WIN32)
    set(CMAKE_FIND_LIBRARY_SUFFIXES "${_gtc_saved_suffixes}")
endif()

if(NOT GEOTRACE_C_LIBRARY)
    message(FATAL_ERROR
        "libgeotrace_c not found. Build it with:\n"
        "  cargo build -p geotrace-c --release\n"
        "or set GEOTRACE_C_LIB_DIR to your install prefix."
    )
endif()

if(GEOTRACE_C_STATIC OR WIN32)
    add_library(GeoTrace::C STATIC IMPORTED GLOBAL)
else()
    add_library(GeoTrace::C SHARED IMPORTED GLOBAL)
endif()

if(WIN32)
    # Rust's static archive on Windows uses NT I/O and Win32 APIs that MSVC
    # doesn't auto-link; consumers must pull these in explicitly.
    set(_gtc_iface_libs ntdll userenv ws2_32 advapi32 bcrypt)
elseif(UNIX AND NOT APPLE AND (GEOTRACE_C_STATIC OR WIN32))
    set(_gtc_iface_libs m dl pthread)
else()
    set(_gtc_iface_libs "")
endif()

set_target_properties(GeoTrace::C PROPERTIES
    IMPORTED_LOCATION "${GEOTRACE_C_LIBRARY}"
    INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_SOURCE_DIR}"
    INTERFACE_LINK_LIBRARIES "${_gtc_iface_libs}"
)

include(GNUInstallDirs)

install(FILES geotrace.h
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}"
)

# Static libraries (and Windows, which is always static) are plain files.
# Shared libraries on Unix use IMPORTED_RUNTIME_ARTIFACTS (added in CMake 3.21).
if(GEOTRACE_C_STATIC OR WIN32)
    install(FILES "${GEOTRACE_C_LIBRARY}"
        DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    )
else()
    install(IMPORTED_RUNTIME_ARTIFACTS GeoTrace::C
        DESTINATION "${CMAKE_INSTALL_LIBDIR}"
    )
    # Rust sets the dylib's embedded install name (LC_ID_DYLIB) to the absolute
    # path of the cargo output directory.  Fix it to @rpath/... so the installed
    # dylib is relocatable and consumers can find it via their RPATH.
    if(APPLE)
        install(CODE "
            execute_process(
                COMMAND install_name_tool
                    -id \"@rpath/libgeotrace_c.dylib\"
                    \"\${CMAKE_INSTALL_PREFIX}/${CMAKE_INSTALL_LIBDIR}/libgeotrace_c.dylib\"
            )
        ")
    endif()
endif()

include(CMakePackageConfigHelpers)

# Encode the static/shared choice so the installed GeoTraceCConfig.cmake
# can reconstruct the same IMPORTED target type.
if(GEOTRACE_C_STATIC OR WIN32)
    set(_gtc_config_static "TRUE")
else()
    set(_gtc_config_static "FALSE")
endif()

configure_package_config_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/GeoTraceCConfig.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCConfig.cmake"
    INSTALL_DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/GeoTraceC"
    PATH_VARS CMAKE_INSTALL_LIBDIR CMAKE_INSTALL_INCLUDEDIR
)

write_basic_package_version_file(
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCConfigVersion.cmake"
    VERSION "${PROJECT_VERSION}"
    COMPATIBILITY SameMajorVersion
)

install(FILES
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCConfig.cmake"
    "${CMAKE_CURRENT_BINARY_DIR}/GeoTraceCConfigVersion.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/GeoTraceC"
)

option(GEOTRACE_C_BUILD_EXAMPLES "Build C SDK examples" OFF)
option(GEOTRACE_C_BUILD_TESTS "Build C SDK tests" OFF)

# Enable testing before recursing so the examples can register ctest smoke
# tests even when only GEOTRACE_C_BUILD_EXAMPLES is set.
if(GEOTRACE_C_BUILD_EXAMPLES OR GEOTRACE_C_BUILD_TESTS)
    enable_testing()
endif()

if(GEOTRACE_C_BUILD_EXAMPLES)
    add_subdirectory(examples)
endif()

if(GEOTRACE_C_BUILD_TESTS)
    add_subdirectory(tests)
endif()
