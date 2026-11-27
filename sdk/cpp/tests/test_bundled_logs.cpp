#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <filesystem>
#include <fstream>
#include <ios>
#include <string>
#include <system_error>
#include <utility>
#include <vector>

#if GEOTRACE_CPP_EXCEPTIONS
#include <stdexcept>
#endif

#include "test_timestamps.hpp"

using geotrace::Angle;
using geotrace::BundledLog;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;

#if GEOTRACE_CPP_EXCEPTIONS
using geotrace::BundledLogView;
using geotrace::Error;
using geotrace::IoError;
#endif

namespace {

struct ScopedFile {
    std::filesystem::path path;

    explicit ScopedFile(std::filesystem::path file_path) : path(std::move(file_path)) {}
    ScopedFile(const ScopedFile &) = delete;
    ScopedFile &operator=(const ScopedFile &) = delete;
    ScopedFile(ScopedFile &&) = delete;
    ScopedFile &operator=(ScopedFile &&) = delete;

    ~ScopedFile() {
        std::error_code ignored;
        std::filesystem::remove(path, ignored);
    }
};

} // namespace

TEST_CASE("bundled logs: multiple logs round-trip sorted by name") {
    const std::string zeta_text = "May 29 18:48:24 host proc: started\n";
    const std::string alpha_text = "2026-05-29T18:48:24Z boot complete\n";
    const std::string nul_text = std::string{"2026-05-29 18:48:24 line1"} + '\0' + "after_nul\n";

    std::vector<std::uint8_t> bytes;
    {
        const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(51.5),
                         Angle::degrees(-0.1)};
        FileBuilder builder;
        builder.add_nav_fix(fix)
            .add_bundled_log(BundledLog{"zeta.log", zeta_text})
            .add(BundledLog{"alpha.log", alpha_text})
            .add_bundled_log(BundledLog{"nul.log", nul_text});

        auto built = builder.try_finish();
        REQUIRE(built.is_ok());
        auto encoded = built.value().try_to_bytes();
        REQUIRE(encoded.is_ok());
        bytes = std::move(encoded.value());
    }

    auto reloaded_res = NavFile::try_from_bytes(bytes);
    REQUIRE(reloaded_res.is_ok());
    const NavFile &file = reloaded_res.value();
    REQUIRE(file.bundled_log_count() == 3);

    auto first = file.try_bundled_log(0);
    REQUIRE(first.is_ok());
    CHECK(first.value().name == "alpha.log");
    CHECK(first.value().text == alpha_text);

    auto second = file.try_bundled_log(1);
    REQUIRE(second.is_ok());
    CHECK(second.value().name == "nul.log");
    CHECK(second.value().text == nul_text);

    auto third = file.try_bundled_log(2);
    REQUIRE(third.is_ok());
    CHECK(third.value().name == "zeta.log");
    CHECK(third.value().text == zeta_text);

    auto out_of_range = file.try_bundled_log(3);
    REQUIRE(out_of_range.is_err());
    CHECK(out_of_range.error().code == GTD_ERR_OUT_OF_RANGE);
}

TEST_CASE("bundled logs: add_bundled_log_from_file reads a log from disk") {
#if GEOTRACE_CPP_EXCEPTIONS
    const ScopedFile temp_log{"cpp_bundled_log_from_file_test.log"};
#else
    const ScopedFile temp_log{"cpp_bundled_log_from_file_no_exceptions_test.log"};
#endif
    const std::string content = "2026-05-29 18:48:24 daemon started\n";
    {
        std::ofstream stream{temp_log.path, std::ios::binary};
        REQUIRE(stream.is_open());
        stream.write(content.data(), static_cast<std::streamsize>(content.size()));
        REQUIRE(stream.good());
    }

    FileBuilder builder;
    builder.add_bundled_log_from_file(temp_log.path);
    auto built = builder.try_finish();
    REQUIRE(built.is_ok());

    auto encoded = built.value().try_to_bytes();
    REQUIRE(encoded.is_ok());
    auto reloaded = NavFile::try_from_bytes(encoded.value());
    REQUIRE(reloaded.is_ok());

    REQUIRE(reloaded.value().bundled_log_count() == 1);
    auto log = reloaded.value().try_bundled_log(0);
    REQUIRE(log.is_ok());
    CHECK(log.value().name == temp_log.path.filename().string());
    CHECK(log.value().text == content);
}

#if GEOTRACE_CPP_EXCEPTIONS
TEST_CASE("bundled logs: throwing API reports validation, IO, duplicate and OOB errors") {
    const std::string valid_text = "2026-05-29T18:48:24Z entry\n";

    SUBCASE("empty text") {
        CHECK_THROWS_AS(FileBuilder{}.add_bundled_log(BundledLog{"empty.log", ""}),
                        std::invalid_argument);
    }
    SUBCASE("unrecognised format") {
        CHECK_THROWS_AS(
            FileBuilder{}.add_bundled_log(BundledLog{"plain.log", "no timestamp here\n"}),
            std::invalid_argument);
    }
    SUBCASE("invalid name") {
        CHECK_THROWS_AS(FileBuilder{}.add_bundled_log(BundledLog{"sub/name.log", valid_text}),
                        std::invalid_argument);
    }
    SUBCASE("embedded nul byte in name") {
        const std::string name_with_nul = std::string{"bad"} + '\0' + ".log";
        CHECK_THROWS_AS(FileBuilder{}.add_bundled_log(BundledLog{name_with_nul, valid_text}),
                        std::invalid_argument);
    }
    SUBCASE("invalid UTF-8") {
        const std::string bad_utf8{"\xff\xfe", 2};
        CHECK_THROWS_AS(FileBuilder{}.add_bundled_log(BundledLog{"bad.log", bad_utf8}), Error);
    }
    SUBCASE("missing file") {
        CHECK_THROWS_AS(FileBuilder{}.add_bundled_log_from_file("does_not_exist_cpp_bundled.log"),
                        IoError);
    }
    SUBCASE("duplicate name rejected on finish") {
        FileBuilder builder;
        builder.add_bundled_log(BundledLog{"dup.log", valid_text})
            .add_bundled_log(BundledLog{"dup.log", valid_text});
        CHECK_THROWS_AS(static_cast<void>(builder.finish()), std::invalid_argument);
    }
    SUBCASE("out of range index") {
        auto file = FileBuilder{}.add_bundled_log(BundledLog{"a.log", valid_text}).finish();
        const BundledLogView first = file.bundled_log(0);
        CHECK(first.name == "a.log");
        CHECK_THROWS_AS(static_cast<void>(file.bundled_log(1)), std::out_of_range);
    }
}
#else
TEST_CASE("bundled logs: no-exceptions API records validation, IO and duplicate errors") {
    const std::string valid_text = "2026-05-29T18:48:24Z entry\n";

    {
        FileBuilder builder;
        builder.add_bundled_log(BundledLog{"empty.log", ""});
        CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
    }
    {
        FileBuilder builder;
        const std::string bad_utf8{"\xff\xfe", 2};
        builder.add_bundled_log(BundledLog{"bad.log", bad_utf8});
        CHECK(builder.status().code == GTD_ERR_UTF8);
    }
    {
        FileBuilder builder;
        builder.add_bundled_log_from_file("does_not_exist_cpp_bundled.log");
        CHECK(builder.status().code == GTD_ERR_IO);
    }
    {
        FileBuilder builder;
        builder.add_bundled_log(BundledLog{"dup.log", valid_text})
            .add_bundled_log(BundledLog{"dup.log", valid_text});
        auto res = builder.try_finish();
        REQUIRE(res.is_err());
        CHECK(res.error().code == GTD_ERR_INVALID_ARGUMENT);
    }
}
#endif
