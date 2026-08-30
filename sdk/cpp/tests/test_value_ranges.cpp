#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cmath>

using geotrace::NavFile;

#ifdef GTD_OUT_OF_RANGE_FIXTURE_PATH
TEST_CASE("NavFile: out-of-range coordinates read verbatim") {
    auto file = NavFile::open(GTD_OUT_OF_RANGE_FIXTURE_PATH);
    REQUIRE(file.nav_point_count() == 4);

    CHECK(std::isnan(file.nav_point(0).lat.as_degrees()));
    CHECK(file.nav_point(1).lat.as_degrees() == doctest::Approx(91.0));
    CHECK(file.nav_point(2).lon.as_degrees() == doctest::Approx(-181.0));

    const auto heading = file.nav_point(3).heading;
    REQUIRE(heading.has_value());
    CHECK(heading->as_degrees() == doctest::Approx(675.0));
}
#endif
