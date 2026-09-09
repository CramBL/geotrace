#ifndef GEOTRACE_TEST_UNDECLARED_ENUMS_HPP
#define GEOTRACE_TEST_UNDECLARED_ENUMS_HPP

#include <cstdint>

constexpr std::uint8_t UNDECLARED_CODE = 200;

// A cast is the only way to hold a value no enumerator of `Enumeration`
// declares.
template <typename Enumeration> constexpr Enumeration undeclared_enum_value() noexcept {
    // NOLINTNEXTLINE(clang-analyzer-optin.core.EnumCastOutOfRange)
    return static_cast<Enumeration>(UNDECLARED_CODE);
}

#endif // GEOTRACE_TEST_UNDECLARED_ENUMS_HPP
