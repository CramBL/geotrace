#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <algorithm>
#include <string>
#include <string_view>
#include <vector>

using geotrace::Angle;
using geotrace::Constellation;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::LogLevel;
using geotrace::NavFix;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::Timestamp;

namespace {

constexpr Timestamp FIX_TIME{1'700'000'000'000'000};

struct Record {
    LogLevel level = LogLevel::Error;
    std::string target;
    std::string message;
};

// One fix and one report whose satellites have a PRN of 0 and an SNR of 99
// dB-Hz: the two data quality issues the builder reports at finish.
void build_a_file_the_builder_warns_about() {
    const NavFix fix{FixTime::receiver(FIX_TIME), Angle::degrees(51.5), Angle::degrees(-0.1)};
    const SatelliteReport report{
        FixTime::receiver(FIX_TIME),
        {
            Satellite{Constellation::Gps, 0, true, 45.0F, 90.0F, 40.0F},
            Satellite{Constellation::Gps, 5, true, 30.0F, 120.0F, 99.0F},
        },
    };
    static_cast<void>(FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish());
}

// One fix and one satellite report 1.5 s later, past the association window:
// the builder reports the ghost nav fix it creates for the report, at
// LogLevel::Debug.
void build_a_file_with_a_ghost_fix() {
    const NavFix fix{FixTime::receiver(FIX_TIME), Angle::degrees(51.5), Angle::degrees(-0.1)};
    const SatelliteReport report{
        FixTime::receiver(Timestamp{FIX_TIME.unix_micros + 1'500'000}),
        {Satellite{Constellation::Gps, 5, true, 30.0F, 120.0F, 40.0F}},
    };
    static_cast<void>(FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish());
}

bool a_message_contains(const std::vector<Record> &records, std::string_view needle) {
    return std::any_of(records.begin(), records.end(), [needle](const Record &record) {
        return record.message.find(needle) != std::string::npos;
    });
}

} // namespace

TEST_CASE("set_log_callback: the callback receives the builder warnings") {
    std::vector<Record> records;
    geotrace::set_log_callback(
        [&records](LogLevel level, std::string_view target, std::string_view message) {
            records.push_back(Record{level, std::string{target}, std::string{message}});
        });

    build_a_file_the_builder_warns_about();
    geotrace::clear_log_callback();

    REQUIRE(records.size() == 2);
    CHECK(records.front().level == LogLevel::Warn);
    CHECK(records.front().target == "geotrace_sdk::builder");
    CHECK(a_message_contains(records, "PRN 0"));
    CHECK(a_message_contains(records, "99 dB-Hz"));
}

TEST_CASE("clear_log_callback: a cleared callback receives nothing") {
    std::vector<Record> records;
    geotrace::set_log_callback(
        [&records](LogLevel level, std::string_view target, std::string_view message) {
            records.push_back(Record{level, std::string{target}, std::string{message}});
        });
    geotrace::clear_log_callback();

    build_a_file_the_builder_warns_about();

    CHECK(records.empty());
}

TEST_CASE("set_log_callback: a second callback replaces the first") {
    std::vector<Record> first;
    std::vector<Record> second;
    geotrace::set_log_callback(
        [&first](LogLevel level, std::string_view target, std::string_view message) {
            first.push_back(Record{level, std::string{target}, std::string{message}});
        });
    geotrace::set_log_callback(
        [&second](LogLevel level, std::string_view target, std::string_view message) {
            second.push_back(Record{level, std::string{target}, std::string{message}});
        });

    build_a_file_the_builder_warns_about();
    geotrace::clear_log_callback();

    CHECK(first.empty());
    CHECK(second.size() == 2);
}

TEST_CASE("set_log_level: the default level drops a debug record") {
    std::vector<Record> records;
    geotrace::set_log_callback(
        [&records](LogLevel level, std::string_view target, std::string_view message) {
            records.push_back(Record{level, std::string{target}, std::string{message}});
        });

    build_a_file_with_a_ghost_fix();
    geotrace::clear_log_callback();

    CHECK(records.empty());
}

TEST_CASE("set_log_level: the debug level forwards a debug record") {
    std::vector<Record> records;
    geotrace::set_log_callback(
        [&records](LogLevel level, std::string_view target, std::string_view message) {
            records.push_back(Record{level, std::string{target}, std::string{message}});
        });
    geotrace::set_log_level(LogLevel::Debug);

    build_a_file_with_a_ghost_fix();
    geotrace::clear_log_callback();
    geotrace::set_log_level(LogLevel::Warn);

    REQUIRE(records.size() == 1);
    CHECK(records.front().level == LogLevel::Debug);
    CHECK(a_message_contains(records, "ghost nav fix"));
}
