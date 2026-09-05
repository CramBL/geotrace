#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <string>

#include "test_odr_header_values.hpp"

std::string header_values_from_second_translation_unit() {
    geotrace::FileBuilder builder;
    builder.add_nav_fix(
        geotrace::NavFix{geotrace::FixTime::receiver(geotrace::Timestamp{kOdrFixTimeMicros})});
    const geotrace::NavFile file = builder.finish();
    const GtdTimestamp gps_time = geotrace::detail::to_c(file.nav_point(0).gps_time);
    const geotrace::Result<geotrace::NavFile> missing =
        geotrace::NavFile::try_open(kOdrMissingFilePath);
    return std::to_string(file.nav_point_count()) + " " + std::to_string(gps_time.unix_micros) +
           " " + std::string{geotrace::travel_mode_name(geotrace::TravelMode::Bicycle)} + " " +
           std::to_string(static_cast<int>(missing.error().code));
}
