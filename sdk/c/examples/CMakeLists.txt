cmake_minimum_required(VERSION 3.20)

foreach(example write_basic read_file)
    add_executable(${example} ${example}.c)
    target_link_libraries(${example} PRIVATE GeoTrace::C)
endforeach()
