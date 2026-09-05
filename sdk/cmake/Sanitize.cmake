# cargo builds `GeoTrace::C` outside these projects, so these options never
# reach it.

set(GEOTRACE_SANITIZE "" CACHE STRING
    "Sanitizer for the tests and examples: asan (address + undefined), or empty")

# Every test and example target links this PRIVATE. It has the compile and link
# options that `GEOTRACE_SANITIZE` selects.
add_library(geotrace_sanitizers INTERFACE)

# NAME=value entries for the ENVIRONMENT property of every registered test.
set(GEOTRACE_SANITIZE_TEST_ENVIRONMENT "")

if(NOT GEOTRACE_SANITIZE STREQUAL "")
    if(MSVC)
        message(FATAL_ERROR
            "GEOTRACE_SANITIZE is set to \"${GEOTRACE_SANITIZE}\", which MSVC does not accept: "
            "it spells its sanitizer flags differently."
        )
    elseif(GEOTRACE_SANITIZE STREQUAL "asan")
        # `ctest` passes even when the sanitizer reports undefined behavior,
        # unless -fno-sanitize-recover is set: without it the report leaves the
        # exit status at 0.
        target_compile_options(geotrace_sanitizers INTERFACE
            -fsanitize=address,undefined
            -fno-sanitize-recover=undefined
            -fno-omit-frame-pointer
        )
        target_link_options(geotrace_sanitizers INTERFACE -fsanitize=address,undefined)
        set(GEOTRACE_SANITIZE_TEST_ENVIRONMENT
            "ASAN_OPTIONS=detect_stack_use_after_return=1:strict_string_checks=1:check_initialization_order=1:strict_init_order=1"
            "UBSAN_OPTIONS=print_stacktrace=1"
        )
    else()
        message(FATAL_ERROR
            "GEOTRACE_SANITIZE is set to \"${GEOTRACE_SANITIZE}\"; expected asan or an empty value."
        )
    endif()
endif()
