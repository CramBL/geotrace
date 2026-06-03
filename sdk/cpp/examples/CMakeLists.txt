cmake_minimum_required(VERSION 3.21)

foreach(example write_basic read_file)
    add_executable(${example} ${example}.cpp)
    target_link_libraries(${example} PRIVATE GeoTrace::Cpp)
    target_compile_options(${example} PRIVATE -Wall -Wextra)
endforeach()
