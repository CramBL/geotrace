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
    return NavFix{FixTime::receiver(Timestamp::from_seconds(1700000000ULL)), Angle::degrees(51.5),
                  Angle::degrees(-0.1)};
}

EventMarker bad_marker() {
    return EventMarker{"invalid path with spaces!", Timestamp::from_seconds(1700000001ULL)};
}
} // namespace

TEST_CASE("builder accumulates the first error and surfaces it at try_finish") {
    FileBuilder b;
    b.add(one_fix());
    b.add_event_marker(bad_marker()); // records the error, does not throw or abort
    CHECK(b.status().is_err());
    CHECK(b.status().code == GTD_ERR_INVALID_PATH);

    const auto r = b.try_finish();
    CHECK(r.is_err());
    CHECK(r.error().code == GTD_ERR_INVALID_PATH);
}

TEST_CASE("first error wins: a later valid call does not overwrite it") {
    FileBuilder b;
    b.add_event_marker(bad_marker()); // first error
    b.add(one_fix());                 // no-op while in the error state
    CHECK(b.status().code == GTD_ERR_INVALID_PATH);
}

TEST_CASE("a valid build succeeds without exceptions") {
    const auto r = FileBuilder{}.add(one_fix()).try_finish();
    CHECK(r.is_ok());
    CHECK(r.value().nav_point_count() == std::size_t{1});
}

TEST_CASE("try_open reports an error by value, never aborting") {
    const auto r = NavFile::try_open("/no/such/file.gtd");
    CHECK(r.is_err());
    CHECK(r.error().code == GTD_ERR_IO);
    CHECK(r.get_if() == nullptr);
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
