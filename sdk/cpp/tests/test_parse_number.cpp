#include "../examples/parse_number.hpp"

#include <doctest/doctest.h>

#include <array>
#include <clocale>
#include <cstdint>

namespace {

// Names for one comma-decimal locale, one per naming scheme the C libraries the
// SDK builds against accept. Linux and macOS take the first three, Windows the
// last two.
constexpr std::array<const char *, 5> COMMA_DECIMAL_LOCALES = {
    "da_DK.UTF-8", "da_DK.utf8", "da_DK", "da-DK", "Danish_Denmark.1252",
};

// Sets `LC_NUMERIC` to a locale whose decimal separator is a comma and returns
// its name, or `nullptr` when the platform has none of them installed.
const char *set_a_comma_decimal_locale() {
    for (const char *name : COMMA_DECIMAL_LOCALES) {
        if (std::setlocale(LC_NUMERIC, name) != nullptr &&
            *std::localeconv()->decimal_point == ',') {
            return name;
        }
    }
    static_cast<void>(std::setlocale(LC_NUMERIC, "C"));
    return nullptr;
}

} // namespace

TEST_CASE("parse_decimal_double: a decimal point parses under a comma locale") {
    const char *locale = set_a_comma_decimal_locale();
    if (locale == nullptr) {
        MESSAGE("no comma-decimal locale is installed on this platform");
        return;
    }

    const auto value = examples::parse_decimal_double("51.5074");
    static_cast<void>(std::setlocale(LC_NUMERIC, "C"));

    REQUIRE_MESSAGE(value.has_value(), "rejected under ", locale);
    CHECK_MESSAGE(*value == 51.5074, "parsed under ", locale);
}

TEST_CASE("parse_decimal_double: a comma decimal separator is rejected") {
    CHECK_FALSE(examples::parse_decimal_double("51,5074").has_value());
}

TEST_CASE("parse_decimal_double: trailing characters are rejected") {
    CHECK_FALSE(examples::parse_decimal_double("51.5074abc").has_value());
}

TEST_CASE("parse_decimal_double: a decimal number parses to its value") {
    const auto value = examples::parse_decimal_double("-1.25e2");
    REQUIRE(value.has_value());
    CHECK(*value == -125.0);
}

TEST_CASE("parse_decimal_uint32: trailing characters are rejected") {
    CHECK_FALSE(examples::parse_decimal_uint32("12abc").has_value());
}

TEST_CASE("parse_decimal_uint32: a value past the range is rejected") {
    CHECK_FALSE(examples::parse_decimal_uint32("4294967296").has_value());
}

TEST_CASE("parse_decimal_uint32: a decimal number parses to its value") {
    const auto value = examples::parse_decimal_uint32("4294967295");
    REQUIRE(value.has_value());
    CHECK(*value == UINT32_MAX);
}
