#ifndef GEOTRACE_TEST_ODR_HEADER_VALUES_HPP
#define GEOTRACE_TEST_ODR_HEADER_VALUES_HPP

#include <cstdint>
#include <string>

constexpr std::int64_t kOdrFixTimeMicros = 1700000000000000;
constexpr const char *kOdrMissingFilePath = "geotrace-odr-no-such-file.gtd";

std::string header_values_from_first_translation_unit();
std::string header_values_from_second_translation_unit();

#endif // GEOTRACE_TEST_ODR_HEADER_VALUES_HPP
