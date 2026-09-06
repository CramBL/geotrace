/**
 * Timestamp parsing for the gold dataset CSV fixtures, shared by the C++ gold
 * dataset example and sdk/cpp/tests/test_gold_timestamp.cpp.
 */

#ifndef GEOTRACE_GOLD_TIMESTAMP_HPP
#define GEOTRACE_GOLD_TIMESTAMP_HPP

#include <geotrace/geotrace.hpp>

#include <array>
#include <cstddef>
#include <cstdint>
#include <exception>
#include <optional>
#include <string>

namespace gold {

struct YearMonth {
    std::int32_t year;
    std::int32_t month;
};

inline bool is_leap(std::int32_t year) noexcept {
    return (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
}

inline std::int32_t days_in_month(YearMonth date) noexcept {
    static constexpr std::array<std::int32_t, 12> days_per_month = {31, 28, 31, 30, 31, 30,
                                                                    31, 31, 30, 31, 30, 31};
    return (date.month == 2 && is_leap(date.year))
               ? 29
               : days_per_month.at(static_cast<std::size_t>(date.month - 1));
}

/* Parse "YYYY-MM-DDTHH:MM:SS[.ffffff][+HH:MM]" to a Timestamp, or `std::nullopt`
   on failure. An offset that fails to parse is left unapplied, as are
   fractional digits past the sixth. */
inline std::optional<geotrace::Timestamp> parse_timestamp(const std::string &text) {
    if (text.size() < 19) {
        return std::nullopt;
    }
    try {
        const std::int32_t year = std::stoi(text.substr(0, 4));
        const std::int32_t month = std::stoi(text.substr(5, 2));
        const std::int32_t day = std::stoi(text.substr(8, 2));
        const std::int32_t hour = std::stoi(text.substr(11, 2));
        const std::int32_t minute = std::stoi(text.substr(14, 2));
        const std::int32_t second = std::stoi(text.substr(17, 2));
        // Optional fractional seconds (".ffffff"), kept as microseconds.
        std::size_t pos = 19;
        std::int32_t frac_us = 0;
        if (pos < text.size() && text.at(pos) == '.') {
            ++pos;
            std::string digits;
            while (pos < text.size() && text.at(pos) >= '0' && text.at(pos) <= '9') {
                digits.push_back(text.at(pos));
                ++pos;
            }
            digits.resize(6, '0'); // pad / truncate to microseconds
            frac_us = std::stoi(digits);
        }
        // Optional timezone offset ("+HH:MM" / "-HH:MM").
        char sign = '+';
        std::int32_t tz_hours = 0;
        std::int32_t tz_minutes = 0;
        if (pos < text.size() && (text.at(pos) == '+' || text.at(pos) == '-')) {
            sign = text.at(pos);
            if (pos + 6 <= text.size()) {
                tz_hours = std::stoi(text.substr(pos + 1, 2));
                tz_minutes = std::stoi(text.substr(pos + 4, 2));
            }
        }
        std::int64_t days = 0;
        for (std::int32_t y = 1970; y < year; y++) {
            days += is_leap(y) ? 366 : 365;
        }
        for (std::int32_t m = 1; m < month; m++) {
            days += days_in_month(YearMonth{year, m});
        }
        days += day - 1;
        std::int64_t secs =
            (days * INT64_C(86400)) + (hour * INT64_C(3600)) + (minute * INT64_C(60)) + second;
        const std::int64_t tz_seconds =
            ((static_cast<std::int64_t>(tz_hours) * INT64_C(60)) + tz_minutes) * INT64_C(60);
        secs += (sign == '-') ? tz_seconds : -tz_seconds;
        // `from_seconds` throws for a year past the range a timestamp covers,
        // whose microsecond count would overflow std::int64_t.
        const auto whole_seconds = geotrace::Timestamp::from_seconds(secs);
        return geotrace::Timestamp::from_micros(whole_seconds.unix_micros + frac_us);
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

} // namespace gold

#endif // GEOTRACE_GOLD_TIMESTAMP_HPP
