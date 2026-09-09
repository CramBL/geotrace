/* The parsers for the lower-case wire names of the `.gtd` format. Each case
   covers every name of one set, so a mapping that swaps two of them fails. */

#include "../geotrace.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <string.h>

Test(wire_names, every_constellation_name_parses_to_its_constellation) {
    static const struct {
        const char *name;
        GtdConstellation constellation;
    } cases[] = {
        {"gps", GTD_CONSTELLATION_GPS},         {"glonass", GTD_CONSTELLATION_GLONASS},
        {"galileo", GTD_CONSTELLATION_GALILEO}, {"beidou", GTD_CONSTELLATION_BEIDOU},
        {"navic", GTD_CONSTELLATION_NAVIC},     {"qzss", GTD_CONSTELLATION_QZSS},
    };

    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        GtdConstellation parsed;
        cr_assert_eq(gtd_constellation_from_name(cases[i].name, &parsed), GTD_OK);
        cr_assert_eq(parsed, cases[i].constellation);
    }
}

Test(wire_names, a_constellation_name_outside_the_set_is_a_parse_error) {
    GtdConstellation parsed;
    cr_assert_eq(gtd_constellation_from_name("pulsar", &parsed), GTD_ERR_PARSE);
    cr_assert_not_null(strstr(gtd_last_error(), "pulsar"));
}

Test(wire_names, every_marker_icon_name_parses_to_its_icon) {
    static const struct {
        const char *name;
        GtdMarkerIcon icon;
    } cases[] = {
        {"pin", GTD_ICON_PIN},
        {"cross", GTD_ICON_CROSS},
        {"circle", GTD_ICON_CIRCLE},
        {"lightning", GTD_ICON_LIGHTNING},
        {"warning", GTD_ICON_WARNING},
        {"error", GTD_ICON_ERROR},
        {"check", GTD_ICON_CHECK},
        {"satellite", GTD_ICON_SATELLITE},
        {"satellite_lost", GTD_ICON_SATELLITE_LOST},
        {"gear", GTD_ICON_GEAR},
        {"refresh", GTD_ICON_REFRESH},
        {"download", GTD_ICON_DOWNLOAD},
        {"upload", GTD_ICON_UPLOAD},
        {"wrench", GTD_ICON_WRENCH},
    };

    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        GtdMarkerIcon parsed;
        cr_assert_eq(gtd_marker_icon_from_name(cases[i].name, &parsed), GTD_OK);
        cr_assert_eq(parsed, cases[i].icon);
    }
}

/* `GTD_ICON_AUTO` is the one `GtdMarkerIcon` value with no wire name. */
Test(wire_names, a_marker_icon_name_outside_the_set_is_a_parse_error) {
    GtdMarkerIcon parsed;
    cr_assert_eq(gtd_marker_icon_from_name("compass", &parsed), GTD_ERR_PARSE);
    cr_assert_not_null(strstr(gtd_last_error(), "compass"));
    cr_assert_eq(gtd_marker_icon_from_name("auto", &parsed), GTD_ERR_PARSE);
}
