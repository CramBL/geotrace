#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <type_traits>
#include <utility>

using geotrace::Angle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Result;
using geotrace::Timestamp;
using geotrace::Velocity;

static_assert(!std::is_default_constructible_v<NavFile>);
static_assert(!std::is_copy_constructible_v<NavFile>);
static_assert(std::is_move_constructible_v<NavFile>);

static_assert(!std::is_copy_constructible_v<FileBuilder>);
static_assert(std::is_move_constructible_v<FileBuilder>);

// A `Result` takes the copy and move semantics of the type it holds.
static_assert(!std::is_copy_constructible_v<Result<NavFile>>);
static_assert(std::is_move_constructible_v<Result<NavFile>>);

static_assert(std::is_trivially_copyable_v<Timestamp>);
static_assert(std::is_trivially_copyable_v<FixTime>);
static_assert(std::is_trivially_copyable_v<Angle>);
static_assert(std::is_trivially_copyable_v<Velocity>);

TEST_CASE("a NavFile moves out of the Result that holds it") {
    const NavFix fix{FixTime::receiver(Timestamp::from_seconds(1700000000)), Angle::degrees(51.5),
                     Angle::degrees(-0.1)};
    Result<NavFile> built = FileBuilder{}.add_nav_fix(fix).try_finish();
    REQUIRE(built.is_ok());

    const NavFile file = std::move(built).value_or_throw();
    CHECK(file.nav_point_count() == 1);
}
