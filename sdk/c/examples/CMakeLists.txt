cmake_minimum_required(VERSION 3.21)

set(GEOTRACE_C_EXAMPLES
    write_basic
    read_file
    with_satellites
    event_markers
    channels
    from_csv
    from_multiple_sources
    gold_dataset
)

foreach(example ${GEOTRACE_C_EXAMPLES})
    add_executable(${example} ${example}.c)
    target_link_libraries(${example} PRIVATE GeoTrace::C)

    if(MSVC)
        target_compile_options(${example} PRIVATE /W4 /WX
            /D_CRT_SECURE_NO_WARNINGS
            /wd4702  # unreachable code: dummy returns after exit() in FAILF macro
        )
    else()
        target_compile_options(${example} PRIVATE
            -Wall -Wextra -Wpedantic
            -Wshadow -Wstrict-prototypes -Wcast-qual -Wformat=2
            -Wmissing-declarations -Wconversion
            -Werror
        )
    endif()
endforeach()

# Register self-contained examples as `ctest` smoke tests so CI runs them.
# `gold_dataset` needs the CSV fixtures and writes into the source tree, so it is
# exercised separately by the `test-gold-c` recipe and excluded here.
foreach(example write_basic with_satellites event_markers channels from_csv from_multiple_sources)
    add_test(NAME example_${example} COMMAND ${example})
endforeach()

# `read_file` takes a .gtd path. Feed it the minimal fixture the unit tests use.
set(GTD_MINIMAL_FIXTURE "${CMAKE_CURRENT_SOURCE_DIR}/../tests/fixtures/minimal.gtd")
if(EXISTS "${GTD_MINIMAL_FIXTURE}")
    add_test(NAME example_read_file COMMAND read_file "${GTD_MINIMAL_FIXTURE}")
endif()
