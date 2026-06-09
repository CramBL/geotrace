cmake_minimum_required(VERSION 3.21)

foreach(example write_basic read_file gold_dataset)
    add_executable(${example} ${example}.c)
    target_link_libraries(${example} PRIVATE GeoTrace::C)

    if(MSVC)
        target_compile_options(${example} PRIVATE /W4 /WX /D_CRT_SECURE_NO_WARNINGS)
    else()
        target_compile_options(${example} PRIVATE
            -Wall -Wextra -Wpedantic
            -Wshadow -Wstrict-prototypes -Wcast-qual -Wformat=2
            -Werror
        )
    endif()
endforeach()
