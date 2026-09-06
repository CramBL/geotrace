// Built with -DGEOTRACE_CPP_NO_EXCEPTIONS to exercise the non-throwing,
// sticky-error path of the SDK. doctest itself still uses exceptions. Only the
// GeoTrace header is forced onto its no-exceptions branch.
#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstddef>

#if GEOTRACE_CPP_EXCEPTIONS
#error "this translation unit must be built with GEOTRACE_CPP_NO_EXCEPTIONS"
#endif

using geotrace::Angle;
using geotrace::EventMarker;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Timestamp;

namespace {
NavFix one_fix() {
    return NavFix{FixTime::receiver(Timestamp::from_seconds(1700000000)), Angle::degrees(51.5),
                  Angle::degrees(-0.1)};
}

EventMarker bad_marker() {
    return EventMarker{"invalid path with spaces!", Timestamp::from_seconds(1700000001)};
}
} // namespace

TEST_CASE("builder accumulates the first error and surfaces it at try_finish") {
    FileBuilder builder;
    builder.add(one_fix());
    builder.add_event_marker(bad_marker()); // records the error, does not throw or abort
    CHECK(builder.status().is_err());
    CHECK(builder.status().code == GTD_ERR_INVALID_PATH);

    const auto result = builder.try_finish();
    CHECK(result.is_err());
    CHECK(result.error().code == GTD_ERR_INVALID_PATH);
}

TEST_CASE("lenient() after a nav fix records the call-order status") {
    FileBuilder builder;
    builder.add(one_fix());
    builder.lenient();
    CHECK(builder.status().code == GTD_ERR_CALL_ORDER);
}

TEST_CASE("first error wins: a later valid call does not overwrite it") {
    FileBuilder builder;
    builder.add_event_marker(bad_marker()); // first error
    builder.add(one_fix());                 // no-op while in the error state
    CHECK(builder.status().code == GTD_ERR_INVALID_PATH);
}

TEST_CASE("a valid build succeeds without exceptions") {
    const auto result = FileBuilder{}.add(one_fix()).try_finish();
    CHECK(result.is_ok());
    CHECK(result.value().nav_point_count() == std::size_t{1});
}

TEST_CASE("try_open reports an error by value, never aborting") {
    const auto result = NavFile::try_open("/no/such/file.gtd");
    CHECK(result.is_err());
    CHECK(result.error().code == GTD_ERR_IO);
    CHECK(result.get_if() == nullptr);
}
using geotrace::ChannelUnit;

TEST_CASE("unit factories report invalid user input without terminating") {
    CHECK(ChannelUnit::try_custom("\xC2\xA0").is_err());
    CHECK(ChannelUnit::try_custom("bad\xC2\x85unit").is_err());
    CHECK(ChannelUnit::try_custom("m/s\xC2\xB2").is_err());
    CHECK(ChannelUnit::try_parse_recognized("unknown").is_err());

    const auto custom = ChannelUnit::try_custom("rotations");
    REQUIRE(custom.is_ok());
    CHECK(custom.value().label() == "rotations");
}
