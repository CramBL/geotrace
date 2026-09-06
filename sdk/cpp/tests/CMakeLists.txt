cmake_minimum_required(VERSION 3.21)

include(FetchContent)

FetchContent_Declare(
    doctest
    GIT_REPOSITORY https://github.com/doctest/doctest.git
    GIT_TAG        v2.5.2
    GIT_SHALLOW    TRUE
)

# doctest v2.5.0+ declares cmake_minimum_required(VERSION 3.14), so no policy
# escape hatch is needed for the vendored dependency.
FetchContent_MakeAvailable(doctest)

set(GEOTRACE_CPP_FIXTURE "${CMAKE_CURRENT_SOURCE_DIR}/../../c/tests/fixtures/minimal.gtd")
set(GEOTRACE_CPP_OUT_OF_RANGE_FIXTURE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../c/tests/fixtures/out_of_range_values.gtd")
set(GEOTRACE_CPP_UNRECOGNIZED_MARKER_ICON_FIXTURE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../c/tests/fixtures/unrecognized_marker_icon.gtd")
set(GEOTRACE_CPP_UNRECOGNIZED_STYLE_VALUES_FIXTURE
    "${CMAKE_CURRENT_SOURCE_DIR}/../../c/tests/fixtures/unrecognized_style_values.gtd")

function(add_cpp_test name)
    add_executable(${name} ${name}.cpp ${ARGN})
    target_link_libraries(${name} PRIVATE GeoTrace::Cpp doctest::doctest geotrace_sanitizers)

    if(MSVC)
        # /wd5285: doctest forward-declares std::tuple, which newer MSVC
        # (VS 2026 / cl 14.51) flags as C5285 (specializing a std template).
        # Under /WX that becomes an error, so silence this one third-party
        # diagnostic without weakening warnings for our own code.
        target_compile_options(${name} PRIVATE /W4 /WX /wd4100 /wd5285)
    else()
        target_compile_options(${name} PRIVATE
            -Wall -Wextra -Wpedantic
            -Wshadow -Wnon-virtual-dtor -Woverloaded-virtual
            -Wcast-qual -Wformat=2 -Wmissing-declarations
            -Wconversion -Wold-style-cast
            -Wno-missing-field-initializers
            -Werror
        )
    endif()

    target_compile_definitions(${name} PRIVATE
        DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN
    )
    if(EXISTS "${GEOTRACE_CPP_FIXTURE}")
        target_compile_definitions(${name} PRIVATE
            GTD_FIXTURE_PATH="${GEOTRACE_CPP_FIXTURE}"
        )
    endif()
    if(EXISTS "${GEOTRACE_CPP_OUT_OF_RANGE_FIXTURE}")
        target_compile_definitions(${name} PRIVATE
            GTD_OUT_OF_RANGE_FIXTURE_PATH="${GEOTRACE_CPP_OUT_OF_RANGE_FIXTURE}"
        )
    endif()
    if(EXISTS "${GEOTRACE_CPP_UNRECOGNIZED_MARKER_ICON_FIXTURE}")
        target_compile_definitions(${name} PRIVATE
            GTD_UNRECOGNIZED_MARKER_ICON_FIXTURE_PATH="${GEOTRACE_CPP_UNRECOGNIZED_MARKER_ICON_FIXTURE}"
        )
    endif()
    if(EXISTS "${GEOTRACE_CPP_UNRECOGNIZED_STYLE_VALUES_FIXTURE}")
        target_compile_definitions(${name} PRIVATE
            GTD_UNRECOGNIZED_STYLE_VALUES_FIXTURE_PATH="${GEOTRACE_CPP_UNRECOGNIZED_STYLE_VALUES_FIXTURE}"
        )
    endif()

    # On Unix, embed an RPATH so `ctest` can find the shared library without
    # LD_LIBRARY_PATH / DYLD_LIBRARY_PATH.  On Windows, GeoTrace::C is a
    # static library so there is no DLL to locate at runtime.
    if(NOT WIN32)
        set_target_properties(${name} PROPERTIES
            BUILD_RPATH "$<TARGET_FILE_DIR:GeoTrace::C>"
        )
    endif()

    add_test(NAME ${name} COMMAND ${name})
    set_tests_properties(${name} PROPERTIES
        ENVIRONMENT "${GEOTRACE_SANITIZE_TEST_ENVIRONMENT}"
    )
endfunction()

add_cpp_test(test_file_builder)
add_cpp_test(test_fix_time)
add_cpp_test(test_timestamp)
add_cpp_test(test_special_members)
add_cpp_test(test_nav_file)
add_cpp_test(test_markers)
add_cpp_test(test_round_trip)
add_cpp_test(test_value_ranges)
add_cpp_test(test_channels)
add_cpp_test(test_exceptions)
add_cpp_test(test_event_kind)
add_cpp_test(test_gold_timestamp)
add_cpp_test(test_result_api)
target_compile_definitions(test_result_api PRIVATE NDEBUG)

# Exercise the non-throwing path at runtime by forcing the header onto its
# no-exceptions branch (the compiler keeps exceptions, so doctest still works).
add_cpp_test(test_no_exceptions)
target_compile_definitions(test_no_exceptions PRIVATE GEOTRACE_CPP_NO_EXCEPTIONS)

add_cpp_test(test_unit_catalog_header)

add_cpp_test(test_odr_two_translation_units test_odr_second_translation_unit.cpp)
