/**
 * Number parsing shared by the C++ examples that read numbers out of CSV text.
 */

#ifndef GEOTRACE_EXAMPLES_PARSE_NUMBER_HPP
#define GEOTRACE_EXAMPLES_PARSE_NUMBER_HPP

#include <cstddef>
#include <cstdint>
#include <exception>
#include <optional>
#include <string>
#include <string_view>

namespace examples {

/// The value of `text`, or `std::nullopt` when `text` is not a decimal number.
[[nodiscard]] inline std::optional<double> parse_decimal_double(std::string_view text) {
    try {
        std::size_t pos = 0;
        const double value = std::stod(std::string(text), &pos);
        return (pos > 0) ? std::optional<double>{value} : std::nullopt;
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

/// The value of `text`, or `std::nullopt` when `text` is not a decimal number
/// that a `std::uint32_t` holds.
[[nodiscard]] inline std::optional<std::uint32_t> parse_decimal_uint32(std::string_view text) {
    try {
        const std::uint64_t value = std::stoul(std::string(text));
        if (value > UINT32_MAX) {
            return std::nullopt;
        }
        return static_cast<std::uint32_t>(value);
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

} // namespace examples

#endif // GEOTRACE_EXAMPLES_PARSE_NUMBER_HPP
