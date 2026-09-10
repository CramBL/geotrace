#include "../examples/parse_number.h"
#include <criterion/criterion.h>
#include <locale.h>
#include <stddef.h>

/* Names for one comma-decimal locale, one per naming scheme the C libraries the
   SDK builds against accept. Linux and macOS take the first three, Windows the
   last two. */
static const char *const COMMA_DECIMAL_LOCALES[] = {
    "da_DK.UTF-8", "da_DK.utf8", "da_DK", "da-DK", "Danish_Denmark.1252",
};

/* Sets LC_NUMERIC to a locale whose decimal separator is a comma and returns
   its name, or NULL when the platform has none of them installed. */
static const char *set_a_comma_decimal_locale(void) {
    for (size_t i = 0; i < sizeof COMMA_DECIMAL_LOCALES / sizeof COMMA_DECIMAL_LOCALES[0]; i++) {
        const char *name = COMMA_DECIMAL_LOCALES[i];
        if (setlocale(LC_NUMERIC, name) != NULL && *localeconv()->decimal_point == ',') {
            return name;
        }
    }
    setlocale(LC_NUMERIC, "C");
    return NULL;
}

Test(parse_number, a_decimal_point_parses_under_a_comma_locale) {
    const char *locale = set_a_comma_decimal_locale();
    if (locale == NULL) {
        cr_skip("no comma-decimal locale is installed on this platform");
    }

    double value = 0.0;
    cr_assert(parse_decimal_double("51.5074", &value), "rejected under %s", locale);
    cr_assert_float_eq(value, 51.5074, 0.0, "parsed to %.4f under %s", value, locale);
}

Test(parse_number, a_comma_decimal_separator_is_rejected) {
    double value = 0.0;
    cr_assert_not(parse_decimal_double("51,5074", &value));
}

Test(parse_number, trailing_characters_are_rejected) {
    double value = 0.0;
    cr_assert_not(parse_decimal_double("51.5074abc", &value));
}

Test(parse_number, a_decimal_number_parses_to_its_value) {
    double value = 0.0;
    cr_assert(parse_decimal_double("-1.25e2", &value));
    cr_assert_float_eq(value, -125.0, 0.0);
}
