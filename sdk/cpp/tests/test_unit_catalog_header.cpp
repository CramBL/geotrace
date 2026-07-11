#include <geotrace/unit_catalog.hpp>

#include <doctest/doctest.h>

#include <string_view>
#include <type_traits>

static_assert(std::is_enum_v<geotrace::RecognizedUnit>);

TEST_CASE("generated unit catalog header is self-contained") {
    CHECK(std::string_view{geotrace::recognized_unit_label(geotrace::RecognizedUnit::Mg)} == "mg");
}
