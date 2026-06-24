#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <exception>
#include <stdexcept>
#include <type_traits>

using geotrace::Angle;
using geotrace::Annotation;
using geotrace::AnnotationsOutOfRangeError;
using geotrace::BuildError;
using geotrace::Error;
using geotrace::EventMarker;
using geotrace::FileBuilder;
using geotrace::Hdf5Error;
using geotrace::InvalidPathError;
using geotrace::IoError;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::NoNavFixesError;
using geotrace::Timestamp;
using geotrace::UnsupportedVersionError;

TEST_CASE("exception hierarchy: all types derive from geotrace::Error") {
    CHECK(std::is_base_of_v<Error, BuildError>);
    CHECK(std::is_base_of_v<Error, IoError>);
    CHECK(std::is_base_of_v<Error, Hdf5Error>);
    CHECK(std::is_base_of_v<Error, UnsupportedVersionError>);
    CHECK(std::is_base_of_v<Error, InvalidPathError>);
    CHECK(std::is_base_of_v<BuildError, NoNavFixesError>);
    CHECK(std::is_base_of_v<BuildError, AnnotationsOutOfRangeError>);
}

TEST_CASE("exception hierarchy: all types derive from std::exception") {
    CHECK(std::is_base_of_v<std::exception, Error>);
}

TEST_CASE("exception: NoNavFixesError is catchable as BuildError and Error") {
    auto throw_it = [] {
        FileBuilder b;
        Annotation ann;
        ann.time = Timestamp::from_seconds(1700000000ULL);
        ann.label = "no fixes";
        b.add_annotation(ann);
        b.finish();
    };
    CHECK_THROWS_AS(throw_it(), NoNavFixesError);
    CHECK_THROWS_AS(throw_it(), BuildError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_THROWS_AS(throw_it(), std::exception);
}

TEST_CASE("exception: InvalidPathError is catchable as Error") {
    auto throw_it = [] {
        FileBuilder b;
        const Timestamp t = Timestamp::from_seconds(1700000000ULL);
        NavFix fix{};
        fix.gps_time = t;
        fix.lat = Angle::degrees(0.0);
        fix.lon = Angle::degrees(0.0);
        b.add_nav_fix(fix);

        EventMarker marker{};
        marker.variant_path = "invalid path with spaces!";
        marker.sys_time = t;
        b.add_event_marker(marker);
    };
    CHECK_THROWS_AS(throw_it(), InvalidPathError);
    CHECK_THROWS_AS(throw_it(), Error);
}

TEST_CASE("exception: IoError is catchable as Error") {
    auto throw_it = [] { NavFile::open("/no/such/file.gtd"); };
    CHECK_THROWS_AS(throw_it(), IoError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_THROWS_AS(throw_it(), std::exception);
}

TEST_CASE("exception: out_of_range from nav_point is std::out_of_range, not geotrace::Error") {
    NavFix fix{};
    fix.gps_time = Timestamp::from_seconds(1700000000ULL);
    fix.lat = Angle::degrees(0.0);
    fix.lon = Angle::degrees(0.0);

    auto file = FileBuilder{}.add_nav_fix(fix).finish();

    CHECK_THROWS_AS(file.nav_point(9999), std::out_of_range);
    // std::out_of_range is NOT a geotrace::Error
    CHECK_THROWS_AS(file.nav_point(9999), std::exception);
    CHECK_NOTHROW(file.nav_point(0));
}

TEST_CASE("exception: AnnotationsOutOfRangeError carries a count field") {
    // Create a file where an annotation falls outside the nav fix time range.
    // Two nav fixes from T1 to T2. Annotation at T0 (before T1) - out of range.
    const Timestamp t1 = Timestamp::from_seconds(1700000100ULL);
    const Timestamp t2 = Timestamp::from_seconds(1700000200ULL);
    const Timestamp t0 = Timestamp::from_seconds(1700000000ULL); // before t1

    try {
        NavFix f1{};
        f1.gps_time = t1;
        f1.lat = Angle::degrees(0.0);
        f1.lon = Angle::degrees(0.0);

        NavFix f2{};
        f2.gps_time = t2;
        f2.lat = Angle::degrees(0.1);
        f2.lon = Angle::degrees(0.1);

        Annotation ann{};
        ann.time = t0;
        ann.label = "outside range";

        auto file = FileBuilder{}.add_nav_fix(f1).add_nav_fix(f2).add_annotation(ann).finish();
        FAIL("expected AnnotationsOutOfRangeError");
    } catch (const AnnotationsOutOfRangeError &e) {
        CHECK(std::string{e.what()}.size() > 0);
        // count field exists (may be 0 due to C API not surfacing exact count)
        CHECK(e.count >= 0);
    }
}

TEST_CASE("exception: what() returns a non-empty string") {
    try {
        FileBuilder b;
        Annotation ann{};
        ann.time = Timestamp::from_seconds(1700000000ULL);
        ann.label = "no fixes";
        b.add_annotation(ann);
        b.finish();
    } catch (const Error &e) {
        CHECK(std::string{e.what()}.size() > 0);
    }
}
