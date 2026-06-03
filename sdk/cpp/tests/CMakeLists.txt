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
    target_compile_options(${name} PRIVATE -Wall -Wextra -Werror)
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

    set_target_properties(${name} PROPERTIES
        ENVIRONMENT "LD_LIBRARY_PATH=$<TARGET_FILE_DIR:GeoTrace::C>:$ENV{LD_LIBRARY_PATH}"
    )
    add_test(NAME ${name} COMMAND ${name})
endfunction()

add_cpp_test(test_file_builder)
add_cpp_test(test_nav_file)
add_cpp_test(test_round_trip)
add_cpp_test(test_exceptions)
