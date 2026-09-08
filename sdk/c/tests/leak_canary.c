/* Leaks one allocation inside `libgeotrace_c`, so that the address sanitizer
   prints a leak report whose allocation stack reaches into the Rust library.
   `check_leak_report_symbolization.cmake` runs this program and reads that
   report.

   The entry point comes from the `sanitizer_canary` feature of geotrace-c. This
   file declares it, because `geotrace.h` states the public API alone. */
extern void gtd_leak_allocation_for_sanitizer_canary(void);

int main(void) {
    gtd_leak_allocation_for_sanitizer_canary();
    return 0;
}
