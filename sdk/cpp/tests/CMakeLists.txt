cmake_minimum_required(VERSION 3.21)

include(FetchContent)

FetchContent_Declare(
    doctest
    GIT_REPOSITORY https://github.com/doctest/doctest.git
    GIT_TAG        v2.4.11
    GIT_SHALLOW    TRUE
)
FetchContent_MakeAvailable(doctest)

set(GEOTRACE_CPP_FIXTURE "${CMAKE_CURRENT_SOURCE_DIR}/../../c/tests/fixtures/minimal.gtd")

function(add_cpp_test name)
    add_executable(${name} ${name}.cpp)
    target_link_libraries(${name} PRIVATE GeoTrace::Cpp doctest::doctest)

    if(MSVC)
        target_compile_options(${name} PRIVATE /W4 /WX /wd4100)
    else()
        target_compile_options(${name} PRIVATE
            -Wall -Wextra -Wpedantic
            -Wshadow -Wnon-virtual-dtor -Woverloaded-virtual
            -Wcast-qual -Wformat=2 -Wmissing-declarations
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

    if(DEFINED GEOTRACE_SANITIZE)
        if(GEOTRACE_SANITIZE STREQUAL "asan")
            target_compile_options(${name} PRIVATE -fsanitize=address,undefined)
            target_link_options(${name} PRIVATE -fsanitize=address,undefined)
        elseif(GEOTRACE_SANITIZE STREQUAL "tsan")
            target_compile_options(${name} PRIVATE -fsanitize=thread)
            target_link_options(${name} PRIVATE -fsanitize=thread)
        endif()
    endif()

    # On Unix, embed an RPATH so ctest can find the shared library without
    # LD_LIBRARY_PATH / DYLD_LIBRARY_PATH.  On Windows, GeoTrace::C is a
    # static library so there is no DLL to locate at runtime.
    if(NOT WIN32)
        set_target_properties(${name} PROPERTIES
            BUILD_RPATH "$<TARGET_FILE_DIR:GeoTrace::C>"
        )
    endif()

    add_test(NAME ${name} COMMAND ${name})
endfunction()

add_cpp_test(test_file_builder)
add_cpp_test(test_nav_file)
add_cpp_test(test_round_trip)
add_cpp_test(test_exceptions)
