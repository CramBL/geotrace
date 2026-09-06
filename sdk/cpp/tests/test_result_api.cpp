#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <cstdint>
#include <vector>

using geotrace::Angle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::IoError;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::NavPointView;
using geotrace::Result;
using geotrace::Timestamp;

namespace {
NavFix one_fix() {
    return NavFix{FixTime::receiver(Timestamp::from_seconds(1700000000ULL)), Angle::degrees(51.5),
                  Angle::degrees(-0.1)};
}
} // namespace

TEST_CASE("try_open on a missing file returns an IO error without throwing") {
    const Result<NavFile> result = NavFile::try_open("/no/such/file.gtd");
    CHECK(result.is_err());
    CHECK(result.error().code == GTD_ERR_IO);
    CHECK_FALSE(result.error().description.empty());
}

TEST_CASE("try_from_bytes on garbage returns a data error without throwing") {
    const std::vector<std::uint8_t> junk = {'n', 'o', 'p', 'e'};
    const Result<NavFile> result = NavFile::try_from_bytes(junk);
    CHECK(result.is_err());
    CHECK((result.error().code == GTD_ERR_HDF5 || result.error().code == GTD_ERR_PARSE));
}

TEST_CASE("try_finish builds a valid file and round-trips via Result") {
    const Result<NavFile> built = FileBuilder{}.add(one_fix()).try_finish();
    REQUIRE(built.is_ok());
    const Result<std::vector<std::uint8_t>> bytes = built.value().try_to_bytes();
    REQUIRE(bytes.is_ok());
    const Result<NavFile> reread = NavFile::try_from_bytes(bytes.value());
    CHECK(reread.is_ok());
    CHECK(reread.value().nav_point_count() == std::size_t{1});
}

TEST_CASE("try_nav_point reports out-of-range without throwing; in-range is ok") {
    const NavFile file = FileBuilder{}.add(one_fix()).finish();
    const Result<NavPointView> oob = file.try_nav_point(9999);
    CHECK(oob.is_err());
    CHECK(oob.error().code == GTD_ERR_OUT_OF_RANGE);
    CHECK(file.try_nav_point(0).is_ok());
}

TEST_CASE("value_or_throw rethrows the typed exception on error") {
    CHECK_THROWS_AS(static_cast<void>(NavFile::try_open("/no/such/file.gtd").value_or_throw()),
                    IoError);
}

TEST_CASE("value access on an error is defined in release builds") {
    const auto result = NavFile::try_open("/no/such/file.gtd");
    CHECK(result.get_if() == nullptr);
    CHECK_THROWS_AS(static_cast<void>(result.value()), IoError);
}
