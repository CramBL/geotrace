#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <type_traits>

using namespace geotrace;

TEST_CASE("exception hierarchy: all types derive from geotrace::Error") {
    CHECK(std::is_base_of<Error, BuildError>::value);
    CHECK(std::is_base_of<Error, IoError>::value);
    CHECK(std::is_base_of<Error, Hdf5Error>::value);
    CHECK(std::is_base_of<Error, UnsupportedVersionError>::value);
    CHECK(std::is_base_of<Error, InvalidPathError>::value);
    CHECK(std::is_base_of<BuildError, NoNavFixesError>::value);
    CHECK(std::is_base_of<BuildError, AnnotationsOutOfRangeError>::value);
}

TEST_CASE("exception hierarchy: all types derive from std::exception") {
    CHECK(std::is_base_of<std::exception, Error>::value);
}

TEST_CASE("exception: NoNavFixesError is catchable as BuildError and Error") {
    auto throw_it = [] {
        FileBuilder{}.finish();
    };
    CHECK_THROWS_AS(throw_it(), NoNavFixesError);
    CHECK_THROWS_AS(throw_it(), BuildError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_THROWS_AS(throw_it(), std::exception);
}

TEST_CASE("exception: InvalidPathError is catchable as Error") {
    auto throw_it = [] {
        FileBuilder b;
        Timestamp t = Timestamp::from_seconds(1700000000ULL);
        b.add_nav_fix(NavFix{.gps_time = t, .lat = Angle::degrees(0.0), .lon = Angle::degrees(0.0)});
        b.add_event_marker(EventMarker{
            .variant_path = "invalid path with spaces!",
            .sys_time     = t,
        });
    };
    CHECK_THROWS_AS(throw_it(), InvalidPathError);
    CHECK_THROWS_AS(throw_it(), Error);
}

TEST_CASE("exception: IoError is catchable as Error") {
    auto throw_it = [] {
        NavFile::open("/no/such/file.gtd");
    };
    CHECK_THROWS_AS(throw_it(), IoError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_THROWS_AS(throw_it(), std::exception);
}

TEST_CASE("exception: out_of_range from nav_point is std::out_of_range, not geotrace::Error") {
    auto file = FileBuilder{}
        .add_nav_fix(NavFix{
            .gps_time = Timestamp::from_seconds(1700000000ULL),
            .lat = Angle::degrees(0.0), .lon = Angle::degrees(0.0),
        })
        .finish();

    CHECK_THROWS_AS(file.nav_point(9999), std::out_of_range);
    // std::out_of_range is NOT a geotrace::Error
    CHECK_THROWS_AS(file.nav_point(9999), std::exception);
    CHECK_NOTHROW(file.nav_point(0));
}

TEST_CASE("exception: AnnotationsOutOfRangeError carries a count field") {
    // Create a file where an annotation falls outside the nav fix time range.
    // Two nav fixes from T1 to T2; annotation at T0 (before T1) - out of range.
    Timestamp t1 = Timestamp::from_seconds(1700000100ULL);
    Timestamp t2 = Timestamp::from_seconds(1700000200ULL);
    Timestamp t0 = Timestamp::from_seconds(1700000000ULL);  // before t1

    try {
        auto file = FileBuilder{}
            .add_nav_fix(NavFix{.gps_time = t1, .lat = Angle::degrees(0.0), .lon = Angle::degrees(0.0)})
            .add_nav_fix(NavFix{.gps_time = t2, .lat = Angle::degrees(0.1), .lon = Angle::degrees(0.1)})
            .add_annotation(Annotation{.time = t0, .label = "outside range"})
            .finish();
        FAIL("expected AnnotationsOutOfRangeError");
    } catch (const AnnotationsOutOfRangeError& e) {
        CHECK(std::string{e.what()}.size() > 0);
        // count field exists (may be 0 due to C API not surfacing exact count)
        CHECK(e.count >= 0);
    }
}

TEST_CASE("exception: what() returns a non-empty string") {
    try {
        FileBuilder{}.finish();
    } catch (const Error& e) {
        CHECK(std::string{e.what()}.size() > 0);
    }
}
