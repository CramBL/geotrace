cmake_minimum_required(VERSION 3.21)

foreach(example write_basic read_file gold_dataset)
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
endforeach()
