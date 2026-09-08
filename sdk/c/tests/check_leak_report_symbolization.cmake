# Runs the leak canary and reads the sanitizer's report from its standard error.
# `test_leak_report_contains_a_rust_source_file` in CMakeLists.txt drives this
# script and passes the canary's path as CANARY_PROGRAM.
#
# The report contains sanitizer_canary.rs when the address sanitizer resolves
# the allocation stack into the Rust library. This script reads the report text
# and ignores the canary's exit status: the sanitizer sets that status
# independently of symbolization.
cmake_minimum_required(VERSION 3.21)

execute_process(
    COMMAND "${CANARY_PROGRAM}"
    OUTPUT_VARIABLE canary_output
    ERROR_VARIABLE leak_report
)

if(NOT leak_report MATCHES "sanitizer_canary\\.rs")
    message(FATAL_ERROR
        "A fault inside libgeotrace_c resolves to a module offset: the leak "
        "report contains no Rust source file. The address sanitizer resolves it "
        "to a .rs file and line only when the library itself is built with "
        "-Zsanitizer=address. The ubuntu-asan row of "
        ".github/workflows/ci_sdk.yml does that, under a nightly pinned by "
        "date. Check that row and its nightly first.\n"
        "Report:\n${leak_report}${canary_output}"
    )
endif()
