#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>
#include <geotrace/unit_catalog.hpp>

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <string>
#include <type_traits>
#include <vector>

using geotrace::Angle;
using geotrace::Channel;
using geotrace::ChannelUnit;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::InvalidChannelError;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::recognized_unit_label;
using geotrace::RecognizedUnit;
using geotrace::Timestamp;

static const Timestamp T0 = Timestamp::from_seconds(1700000000ULL);
static const Timestamp T1 = Timestamp::from_seconds(1700000001ULL);

static_assert(!std::is_default_constructible_v<ChannelUnit>);

TEST_CASE("channels: scalar and vector survive write → from_bytes → read") {
    std::vector<std::uint8_t> bytes;
    {
        const NavFix fix{FixTime::receiver(T0), Angle::degrees(51.5), Angle::degrees(-0.1)};

        Channel incline{};
        incline.name = "incline";
        incline.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Deg);
        incline.period = Angle::degrees(360.0);
        incline.description = "boom inclinometer";
        incline.times = {T0, T1};
        incline.values = {1.5, 2.0};

        Channel accel{};
        accel.name = "accel";
        accel.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::G);
        accel.components = {"x", "y", "z"};
        accel.times = {T0, T1};
        accel.values = {0.1, 0.2, 0.98, -0.1, 0.3, 1.02};

        // A bare channel with no unit, period, or description exercises the
        // empty-means-none marshalling on both write and read.
        Channel temp{};
        temp.name = "temp";
        temp.times = {T0};
        temp.values = {20.0};

        auto file =
            FileBuilder{}.add_nav_fix(fix).add_channel(incline).add(accel).add(temp).finish();
        bytes = file.to_bytes();
    }

    auto file = NavFile::from_bytes(bytes);
    REQUIRE(file.channel_count() == 3);

    // Channels sort by name: accel (vector) then incline (scalar).
    auto accel = file.channel(0);
    CHECK(accel.name == "accel");
    CHECK(accel.is_vector());
    REQUIRE(accel.unit.has_value());
    CHECK(accel.unit->label() == "g");
    CHECK(accel.components == std::vector<std::string>{"x", "y", "z"});
    CHECK_FALSE(accel.period.has_value());
    REQUIRE(accel.times.size() == 2);
    CHECK(accel.times.at(1).unix_micros == T1.unix_micros);
    REQUIRE(accel.values.size() == 6);
    CHECK(accel.values.at(0) == doctest::Approx(0.1));
    CHECK(accel.values.at(5) == doctest::Approx(1.02));

    auto incline = file.channel(1);
    CHECK(incline.name == "incline");
    CHECK_FALSE(incline.is_vector());
    CHECK(incline.components.empty());
    CHECK(incline.description == "boom inclinometer");
    REQUIRE(incline.period.has_value());
    CHECK(incline.period->as_degrees() == doctest::Approx(360.0));

    // The bare channel round-trips with all optional fields absent.
    auto temp = file.channel(2);
    CHECK(temp.name == "temp");
    CHECK(!temp.unit.has_value());
    CHECK(temp.description.empty());
    CHECK_FALSE(temp.period.has_value());
}

TEST_CASE("channels: a malformed channel throws InvalidChannelError") {
    SUBCASE("invalid name") {
        Channel ch{};
        ch.name = "Bad Name";
        ch.times = {T0};
        ch.values = {1.0};
        CHECK_THROWS_AS(FileBuilder{}.add_channel(ch), InvalidChannelError);
    }
    SUBCASE("length mismatch") {
        Channel ch{};
        ch.name = "accel";
        ch.times = {T0};
        ch.values = {1.0, 2.0};
        CHECK_THROWS_AS(FileBuilder{}.add_channel(ch), InvalidChannelError);
    }
    SUBCASE("duplicate component label") {
        Channel ch{};
        ch.name = "accel";
        ch.components = {"x", "x"};
        ch.times = {T0};
        ch.values = {1.0, 2.0};
        CHECK_THROWS_AS(FileBuilder{}.add_channel(ch), InvalidChannelError);
    }
    SUBCASE("unrecognized unit") {
        CHECK_THROWS_AS(static_cast<void>(geotrace::ChannelUnit::parse_recognized("rpm")),
                        InvalidChannelError);
    }
    SUBCASE("duplicate channel name at finish") {
        Channel ch{};
        ch.name = "accel";
        ch.times = {T0};
        ch.values = {1.0};
        CHECK_THROWS_AS(static_cast<void>(FileBuilder{}.add_channel(ch).add_channel(ch).finish()),
                        InvalidChannelError);
    }
}

TEST_CASE("channels: a custom unit is an explicit display-only escape hatch") {
    Channel ch{};
    ch.name = "shaft_speed";
    ch.unit = geotrace::ChannelUnit::custom("rpm");
    ch.times = {T0};
    ch.values = {1200.0};

    auto file = NavFile::from_bytes(FileBuilder{}.add_channel(ch).finish().to_bytes());
    auto read = file.channel(0);
    REQUIRE(read.unit.has_value());
    CHECK(read.unit->label() == "rpm");
    CHECK(read.unit->is_custom());
}

TEST_CASE("channels: long custom units round-trip losslessly") {
    const std::string label(159, 'x');
    Channel ch{};
    ch.name = "quality";
    ch.unit = geotrace::ChannelUnit::custom(label);
    ch.times = {T0};
    ch.values = {1.0};

    auto file = NavFile::from_bytes(FileBuilder{}.add_channel(ch).finish().to_bytes());
    auto read = file.channel(0);
    REQUIRE(read.unit.has_value());
    CHECK(read.unit->label() == label);
    CHECK(read.unit->is_custom());
}

TEST_CASE("channels: generated unit catalog exposes every canonical label") {
    std::vector<std::string> labels;
    for (std::uint8_t raw = 0; raw <= static_cast<std::uint8_t>(RecognizedUnit::PerH); ++raw) {
        const auto unit = static_cast<RecognizedUnit>(raw);
        const auto parsed = ChannelUnit::try_parse_recognized(recognized_unit_label(unit));
        REQUIRE(parsed.is_ok());
        labels.push_back(parsed.value().label());
    }
    CHECK(labels.size() == std::size_t{29});
    std::sort(labels.begin(), labels.end());
    CHECK(std::adjacent_find(labels.begin(), labels.end()) == labels.end());
}

TEST_CASE("channels: try_channel reports out-of-range without throwing") {
    auto file = FileBuilder{}.finish();
    CHECK(file.channel_count() == 0);
    auto result = file.try_channel(0);
    CHECK(result.is_err());
}
