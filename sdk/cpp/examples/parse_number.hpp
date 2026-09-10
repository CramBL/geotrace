/**
 * Number parsing shared by the C++ examples that read numbers out of CSV text.
 *
 * `std::from_chars` reads '.' as the decimal separator under every locale.
 * `std::stod` reads the separator from `LC_NUMERIC`. The parse below calls
 * `std::from_chars` and requires its stop position to be the end of the field.
 */

#ifndef GEOTRACE_EXAMPLES_PARSE_NUMBER_HPP
#define GEOTRACE_EXAMPLES_PARSE_NUMBER_HPP

#include <charconv>
#include <cstddef>
#include <iterator>
#include <optional>
#include <string>
#include <system_error>

namespace examples {

/// The value of `text`, or `std::nullopt` when `text` holds anything but one
/// decimal number in the range of `Number`.
template <typename Number>
[[nodiscard]] inline std::optional<Number> parse_decimal(const std::string &text) {
    Number value{};
    const char *const end = std::next(text.data(), static_cast<std::ptrdiff_t>(text.size()));
    const std::from_chars_result parsed = std::from_chars(text.data(), end, value);
    if (parsed.ec != std::errc{} || parsed.ptr != end) {
        return std::nullopt;
    }
    return value;
}

} // namespace examples

#endif // GEOTRACE_EXAMPLES_PARSE_NUMBER_HPP
