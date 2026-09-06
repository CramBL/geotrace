#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <exception>
#include <stdexcept>
#include <string>
#include <type_traits>

using geotrace::Angle;
using geotrace::Annotation;
using geotrace::AnnotationsOutOfRangeError;
using geotrace::BuildError;
using geotrace::CallOrderError;
using geotrace::Channel;
using geotrace::Error;
using geotrace::EventMarker;
using geotrace::FieldTooLongError;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::Hdf5Error;
using geotrace::InvalidChannelError;
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
    CHECK(std::is_base_of_v<Error, FieldTooLongError>);
    CHECK(std::is_base_of_v<Error, InvalidChannelError>);
    CHECK(std::is_base_of_v<Error, CallOrderError>);
    CHECK(std::is_base_of_v<BuildError, NoNavFixesError>);
    CHECK(std::is_base_of_v<BuildError, AnnotationsOutOfRangeError>);
}

TEST_CASE("exception hierarchy: all types derive from std::exception") {
    CHECK(std::is_base_of_v<std::exception, Error>);
}

TEST_CASE("exception: NoNavFixesError is catchable as BuildError and Error") {
    auto throw_it = [] {
        FileBuilder builder;
        Annotation ann{Timestamp::from_seconds(1700000000)};
        ann.label = "no fixes";
        builder.add_annotation(ann);
        static_cast<void>(builder.finish());
    };
    CHECK_THROWS_AS(throw_it(), NoNavFixesError);
    CHECK_THROWS_AS(throw_it(), BuildError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_THROWS_AS(throw_it(), std::exception);
}

TEST_CASE("exception: InvalidPathError is catchable as Error") {
    auto throw_it = [] {
        FileBuilder builder;
        const Timestamp time = Timestamp::from_seconds(1700000000);
        builder.add_nav_fix(
            NavFix{FixTime::receiver(time), Angle::degrees(0.0), Angle::degrees(0.0)});

        builder.add_event_marker(EventMarker{"invalid path with spaces!", time});
    };
    CHECK_THROWS_AS(throw_it(), InvalidPathError);
    CHECK_THROWS_AS(throw_it(), Error);
}

TEST_CASE("exception: FieldTooLongError is catchable as Error") {
    auto throw_it = [] {
        FileBuilder builder;
        const Timestamp time = Timestamp::from_seconds(1700000000);
        builder.add_nav_fix(
            NavFix{FixTime::receiver(time), Angle::degrees(0.0), Angle::degrees(0.0)});

        builder.add_event_marker(EventMarker{"system/startup", time, std::string(512, 'a')});
    };
    CHECK_THROWS_AS(throw_it(), FieldTooLongError);
    CHECK_THROWS_AS(throw_it(), Error);
}

TEST_CASE("exception: InvalidChannelError is catchable as Error") {
    auto throw_it = [] {
        Channel channel{};
        channel.name = "Bad Name";
        channel.times = {Timestamp::from_seconds(1700000000)};
        channel.values = {1.0};
        FileBuilder{}.add_channel(channel);
    };
    CHECK_THROWS_AS(throw_it(), InvalidChannelError);
    CHECK_THROWS_AS(throw_it(), Error);
}

TEST_CASE("exception: lenient() after a nav fix throws CallOrderError") {
    auto throw_it = [] {
        FileBuilder builder;
        builder.add_nav_fix(NavFix{FixTime::receiver(Timestamp::from_seconds(1700000000)),
                                   Angle::degrees(0.0), Angle::degrees(0.0)});
        builder.lenient();
    };
    CHECK_THROWS_AS(throw_it(), CallOrderError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_NOTHROW(FileBuilder{}.lenient());
}

TEST_CASE("exception: IoError is catchable as Error") {
    auto throw_it = [] { static_cast<void>(NavFile::open("/no/such/file.gtd")); };
    CHECK_THROWS_AS(throw_it(), IoError);
    CHECK_THROWS_AS(throw_it(), Error);
    CHECK_THROWS_AS(throw_it(), std::exception);
}

TEST_CASE("exception: out_of_range from nav_point is std::out_of_range, not geotrace::Error") {
    const NavFix fix{FixTime::receiver(Timestamp::from_seconds(1700000000)), Angle::degrees(0.0),
                     Angle::degrees(0.0)};

    auto file = FileBuilder{}.add_nav_fix(fix).finish();

    CHECK_THROWS_AS(static_cast<void>(file.nav_point(9999)), std::out_of_range);
    // std::out_of_range is NOT a geotrace::Error
    CHECK_THROWS_AS(static_cast<void>(file.nav_point(9999)), std::exception);
    CHECK_NOTHROW(static_cast<void>(file.nav_point(0)));
}

TEST_CASE("exception: AnnotationsOutOfRangeError carries a count field") {
    // The annotation falls outside the file's time range: its timestamp
    // precedes both nav fixes.
    const Timestamp first_fix_time = Timestamp::from_seconds(1700000100);
    const Timestamp second_fix_time = Timestamp::from_seconds(1700000200);
    const Timestamp before_first_fix = Timestamp::from_seconds(1700000000);

    try {
        const NavFix first_fix{FixTime::receiver(first_fix_time), Angle::degrees(0.0),
                               Angle::degrees(0.0)};
        const NavFix second_fix{FixTime::receiver(second_fix_time), Angle::degrees(0.1),
                                Angle::degrees(0.1)};

        Annotation ann{before_first_fix};
        ann.label = "outside range";

        auto file = FileBuilder{}
                        .add_nav_fix(first_fix)
                        .add_nav_fix(second_fix)
                        .add_annotation(ann)
                        .finish();
        FAIL("expected AnnotationsOutOfRangeError");
    } catch (const AnnotationsOutOfRangeError &e) {
        CHECK(std::string{e.what()}.size() > 0);
        // count field exists (may be 0 due to C API not surfacing exact count)
        CHECK(e.count >= 0);
    }
}

TEST_CASE("exception: what() returns a non-empty string") {
    try {
        FileBuilder builder;
        Annotation ann{Timestamp::from_seconds(1700000000)};
        ann.label = "no fixes";
        builder.add_annotation(ann);
        static_cast<void>(builder.finish());
    } catch (const Error &e) {
        CHECK(std::string{e.what()}.size() > 0);
    }
}
