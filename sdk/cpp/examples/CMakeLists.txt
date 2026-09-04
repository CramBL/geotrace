cmake_minimum_required(VERSION 3.21)

set(GEOTRACE_CPP_EXAMPLES
    write_basic
    read_file
    with_satellites
    event_markers
    channels
    typed_events
    from_csv
    from_multiple_sources
    gold_dataset
)

foreach(example ${GEOTRACE_CPP_EXAMPLES})
    add_executable(${example} ${example}.cpp)
    target_link_libraries(${example} PRIVATE GeoTrace::Cpp)

    if(MSVC)
        target_compile_options(${example} PRIVATE /W4 /WX /wd4100)
    else()
        target_compile_options(${example} PRIVATE
            -Wall -Wextra -Wpedantic
            -Wshadow -Wnon-virtual-dtor -Woverloaded-virtual
            -Wcast-qual -Wformat=2 -Wmissing-declarations
            -Wconversion -Wold-style-cast
            -Werror
        )
    endif()

    # On Unix, GeoTrace::C is a shared library. Embed an RPATH so the example
    # binaries find it when run via `ctest` without LD_LIBRARY_PATH.
    if(NOT WIN32)
        set_target_properties(${example} PROPERTIES
            BUILD_RPATH "$<TARGET_FILE_DIR:GeoTrace::C>"
        )
    endif()
endforeach()

# Register self-contained examples as `ctest` smoke tests so CI runs them.
# `read_file` needs an input path and `gold_dataset` needs the CSV fixtures and
# writes into the source tree, so both are exercised separately (test-gold-cpp)
# and excluded here.
foreach(example write_basic with_satellites event_markers channels typed_events from_csv
                from_multiple_sources)
    add_test(NAME example_${example} COMMAND ${example})
endforeach()
