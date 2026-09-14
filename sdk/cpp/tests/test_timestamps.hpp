#ifndef GEOTRACE_TEST_TIMESTAMPS_HPP
#define GEOTRACE_TEST_TIMESTAMPS_HPP

#include <geotrace/geotrace.hpp>

#include <chrono>

inline geotrace::Timestamp fix_timestamp() {
    return geotrace::Timestamp::from_micros(1'700'000'000'000'000);
}

inline geotrace::Timestamp after_fix_timestamp(std::chrono::microseconds offset) {
    return geotrace::Timestamp::from_micros(fix_timestamp().as_unix_micros() + offset.count());
}

#endif // GEOTRACE_TEST_TIMESTAMPS_HPP
