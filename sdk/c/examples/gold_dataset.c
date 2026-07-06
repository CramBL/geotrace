/**
 * Gold dataset reference test for the GeoTrace C SDK.
 *
 * Reads the CSV fixtures in tests/fixtures/gold_dataset/, builds a .gtd file,
 * then verifies the round-trip.  Run from the repository root:
 *
 *   ./sdk/c/build/gold/examples/gold_dataset
 */

#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define FAIL(msg)                                                                                  \
    do {                                                                                           \
        fputs("FAIL: " msg "\n", stderr);                                                          \
        exit(1);                                                                                   \
    } while (0)
#define FAILF(fmt, ...)                                                                            \
    do {                                                                                           \
        fprintf(stderr, "FAIL: " fmt "\n", __VA_ARGS__);                                           \
        exit(1);                                                                                   \
    } while (0)
#define CHECK(cond, msg)                                                                           \
    do {                                                                                           \
        if (!(cond))                                                                               \
            FAIL(msg);                                                                             \
    } while (0)

#define CHECK_SDK(call, label)                                                                     \
    do {                                                                                           \
        GtdStatus _s = (call);                                                                     \
        if (_s != GTD_OK)                                                                          \
            FAILF("%s: %s", label, gtd_last_error());                                              \
    } while (0)

#define CSV_BUFSIZE       4096
#define CSV_MAX_COLS      16
#define TS_BUFSIZE        48
#define MAX_SATS          2048
#define SAT_PER_FIX       64
#define MAX_CHANNELS      8
#define MAX_CH_SAMPLES    64
#define MAX_CH_COMPONENTS 8

static void rtrim(char *s) {
    size_t n = strlen(s);
    while (n > 0 && (s[n - 1] == '\r' || s[n - 1] == '\n' || s[n - 1] == ' '))
        s[--n] = '\0';
}

static int split_delim(char *line, char delim, char *cols[], int max) {
    int n = 0;
    char *p = line;
    while (n < max) {
        cols[n++] = p;
        p = strchr(p, delim);
        if (!p)
            break;
        *p++ = '\0';
    }
    return n;
}

static int split_csv(char *line, char *cols[], int max) {
    return split_delim(line, ',', cols, max);
}

static int is_leap(int y) {
    return (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
}

static int month_days(int m, int y) {
    static const int dom[12] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (m == 2 && is_leap(y)) ? 29 : dom[m - 1];
}

/* Parse "YYYY-MM-DDTHH:MM:SS[.ffffff][+HH:MM]" -> GtdTimestamp.
   Returns gtd_ts_none() on failure. */
static GtdTimestamp parse_ts(const char *s) {
    int Y = 0, Mo = 0, D = 0, H = 0, Mi = 0, S = 0, consumed = 0;

    if (!s || *s == '\0')
        return gtd_ts_none();
    if (sscanf(s, "%d-%d-%dT%d:%d:%d%n", &Y, &Mo, &D, &H, &Mi, &S, &consumed) < 6)
        return gtd_ts_none();

    const char *p = s + consumed;

    /* Optional fractional seconds (".ffffff"), kept as microseconds. */
    long frac_us = 0;
    if (*p == '.') {
        p++;
        char digits[7] = "000000";
        int n = 0;
        while (n < 6 && *p >= '0' && *p <= '9')
            digits[n++] = *p++;
        while (*p >= '0' && *p <= '9') /* skip sub-microsecond digits */
            p++;
        frac_us = strtol(digits, NULL, 10);
    }

    /* Optional timezone offset ("+HH:MM" / "-HH:MM"). */
    char sign = '+';
    long tz = 0;
    if (*p == '+' || *p == '-') {
        int tz_h = 0, tz_m = 0;
        sign = *p;
        if (sscanf(p + 1, "%d:%d", &tz_h, &tz_m) == 2)
            tz = (((long)tz_h * 60L) + tz_m) * 60L;
    }

    long days = 0;
    for (int y = 1970; y < Y; y++)
        days += is_leap(y) ? 366 : 365;
    for (int m = 1; m < Mo; m++)
        days += month_days(m, Y);
    days += D - 1;

    long secs = (days * 86400L) + (H * 3600L) + (Mi * 60L) + S;
    secs += (sign == '-') ? tz : -tz;

    return gtd_ts_from_micros(((uint64_t)secs * 1000000ULL) + (uint64_t)frac_us);
}

static GtdOptF64 parse_opt_f64(const char *s) {
    if (!s || *s == '\0')
        return GTD_NONE_F64;
    char *end;
    double v = strtod(s, &end);
    if (end == s)
        return GTD_NONE_F64;
    return GTD_SOME_F64(v);
}

static GtdConstellation parse_constellation(const char *s) {
    if (strcmp(s, "gps") == 0)
        return GTD_CONSTELLATION_GPS;
    if (strcmp(s, "glonass") == 0)
        return GTD_CONSTELLATION_GLONASS;
    if (strcmp(s, "galileo") == 0)
        return GTD_CONSTELLATION_GALILEO;
    if (strcmp(s, "beidou") == 0)
        return GTD_CONSTELLATION_BEIDOU;
    FAILF("unknown constellation: %s", s);
    return GTD_CONSTELLATION_GPS; /* unreachable */
}

static GtdMarkerIcon parse_icon(const char *s) {
    if (!s || *s == '\0')
        return GTD_ICON_AUTO;
    if (strcmp(s, "pin") == 0)
        return GTD_ICON_PIN;
    if (strcmp(s, "cross") == 0)
        return GTD_ICON_CROSS;
    if (strcmp(s, "circle") == 0)
        return GTD_ICON_CIRCLE;
    if (strcmp(s, "lightning") == 0)
        return GTD_ICON_LIGHTNING;
    if (strcmp(s, "warning") == 0)
        return GTD_ICON_WARNING;
    if (strcmp(s, "error") == 0)
        return GTD_ICON_ERROR;
    if (strcmp(s, "check") == 0)
        return GTD_ICON_CHECK;
    if (strcmp(s, "satellite") == 0)
        return GTD_ICON_SATELLITE;
    if (strcmp(s, "satellite_lost") == 0)
        return GTD_ICON_SATELLITE_LOST;
    if (strcmp(s, "gear") == 0)
        return GTD_ICON_GEAR;
    if (strcmp(s, "refresh") == 0)
        return GTD_ICON_REFRESH;
    if (strcmp(s, "download") == 0)
        return GTD_ICON_DOWNLOAD;
    if (strcmp(s, "upload") == 0)
        return GTD_ICON_UPLOAD;
    if (strcmp(s, "wrench") == 0)
        return GTD_ICON_WRENCH;
    return GTD_ICON_AUTO;
}

typedef struct {
    char gps_time[TS_BUFSIZE];
    char sys_time[TS_BUFSIZE];
    GtdSatellite sat;
} SatRow;

static SatRow g_sats[MAX_SATS];
static int g_sat_n = 0;

static FILE *open_csv(const char *base, const char *name) {
    char path[512];
    int n = snprintf(path, sizeof path, "%s/%s", base, name);
    if (n < 0 || (size_t)n >= sizeof path)
        FAIL("path too long");
    FILE *f = fopen(path, "r");
    if (!f)
        FAILF("cannot open: %s", path);
    return f;
}

static void load_meta(GtdFileBuilder *b, const char *base) {
    FILE *f = open_csv(base, "meta.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f) || !fgets(line, sizeof line, f)) {
        fclose(f);
        FAIL("meta.csv: missing data row");
    }
    fclose(f);
    rtrim(line);
    char *cols[CSV_MAX_COLS];
    if (split_csv(line, cols, CSV_MAX_COLS) < 4)
        FAIL("meta.csv: need 4 columns");
    CHECK_SDK(gtd_builder_set_title(b, cols[0]), "set_title");
    CHECK_SDK(gtd_builder_set_device(b, cols[1]), "set_device");
    CHECK_SDK(gtd_builder_set_notes(b, cols[2]), "set_notes");
    CHECK_SDK(gtd_builder_set_identity(b, cols[3]), "set_identity");
}

static void load_event_styles(GtdFileBuilder *b, const char *base) {
    FILE *f = open_csv(base, "event_styles.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f)) {
        fclose(f);
        return;
    }
    while (fgets(line, sizeof line, f)) {
        rtrim(line);
        if (*line == '\0')
            continue;
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 3)
            continue;
        const char *color = (*cols[2] != '\0') ? cols[2] : NULL;
        CHECK_SDK(gtd_builder_add_event_marker_style(b, cols[0], parse_icon(cols[1]), color),
                  "add_event_marker_style");
    }
    fclose(f);
}

static void load_satellites(const char *base) {
    FILE *f = open_csv(base, "satellites.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f)) {
        fclose(f);
        return;
    }
    while (fgets(line, sizeof line, f)) {
        rtrim(line);
        if (*line == '\0')
            continue;
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 8)
            continue;
        if (g_sat_n >= MAX_SATS)
            FAIL("too many satellite rows");

        SatRow *row = &g_sats[g_sat_n++];
        int ng = snprintf(row->gps_time, sizeof row->gps_time, "%s", cols[0]);
        int ns = snprintf(row->sys_time, sizeof row->sys_time, "%s", cols[1]);
        if (ng < 0 || (size_t)ng >= sizeof row->gps_time || ns < 0 ||
            (size_t)ns >= sizeof row->sys_time)
            FAIL("satellite timestamp too long");

        char *prn_end;
        unsigned long prn = strtoul(cols[3], &prn_end, 10);
        if (prn_end == cols[3])
            FAIL("invalid PRN");

        row->sat.constellation = parse_constellation(cols[2]);
        row->sat.prn = (uint32_t)prn;
        row->sat.in_fix = (uint8_t)(strcmp(cols[4], "true") == 0);
        row->sat.elevation_deg = parse_opt_f64(cols[5]);
        row->sat.azimuth_deg = parse_opt_f64(cols[6]);
        row->sat.snr_dbhz = parse_opt_f64(cols[7]);
    }
    fclose(f);
}

static void load_fixes(GtdFileBuilder *b, const char *base) {
    FILE *f = open_csv(base, "fixes.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f)) {
        fclose(f);
        return;
    }
    while (fgets(line, sizeof line, f)) {
        rtrim(line);
        if (*line == '\0')
            continue;
        char *cols[CSV_MAX_COLS];
        /* cols: track_id, gps_time, sys_time, lat, lon, heading_deg, speed_kmh, eph_m */
        if (split_csv(line, cols, CSV_MAX_COLS) < 8)
            continue;

        GtdTimestamp gps_ts = parse_ts(cols[1]);
        GtdTimestamp sys_ts = parse_ts(cols[2]);

        char *end;
        double lat = strtod(cols[3], &end);
        if (end == cols[3])
            FAIL("invalid latitude");
        double lon = strtod(cols[4], &end);
        if (end == cols[4])
            FAIL("invalid longitude");

        GtdOptF64 hdg = parse_opt_f64(cols[5]);
        GtdOptF64 spd = GTD_NONE_F64;
        if (*cols[6] != '\0') {
            double kmh = strtod(cols[6], &end);
            /* Use the same constant-multiply as Rust's MPS_PER_KMH = 1.0/3.6.
               Direct kmh/3.6 differs by 1 ULP for some values (e.g. 23.2). */
            if (end != cols[6])
                spd = GTD_SOME_F64(kmh * (1.0 / 3.6));
        }
        GtdOptF64 eph = parse_opt_f64(cols[7]);

        CHECK_SDK(gtd_builder_add_nav_fix(b, gps_ts, sys_ts, lat, lon, hdg, spd, eph),
                  "add_nav_fix");

        /* collect satellites for this (gps_time, sys_time) key */
        GtdSatellite sat_buf[SAT_PER_FIX];
        int sat_n = 0;
        for (int i = 0; i < g_sat_n; i++) {
            if (strcmp(g_sats[i].gps_time, cols[1]) == 0 &&
                strcmp(g_sats[i].sys_time, cols[2]) == 0) {
                if (sat_n < SAT_PER_FIX)
                    sat_buf[sat_n++] = g_sats[i].sat;
            }
        }
        if (sat_n > 0) {
            CHECK_SDK(gtd_builder_add_satellite_report(b, gps_ts, sys_ts, sat_buf, (size_t)sat_n),
                      "add_satellite_report");
        }
    }
    fclose(f);
}

static void load_markers(GtdFileBuilder *b, const char *base) {
    FILE *f = open_csv(base, "markers.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f)) {
        fclose(f);
        return;
    }
    while (fgets(line, sizeof line, f)) {
        rtrim(line);
        if (*line == '\0')
            continue;
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 3)
            continue;
        GtdTimestamp ts = parse_ts(cols[0]);
        if (gtd_ts_is_none(ts))
            FAIL("markers.csv: missing timestamp");
        const char *label = (*cols[1] != '\0') ? cols[1] : NULL;
        CHECK_SDK(gtd_builder_add_annotation(b, ts, label, parse_icon(cols[2])), "add_annotation");
    }
    fclose(f);
}

static void load_events(GtdFileBuilder *b, const char *base) {
    FILE *f = open_csv(base, "events.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f)) {
        fclose(f);
        return;
    }
    while (fgets(line, sizeof line, f)) {
        rtrim(line);
        if (*line == '\0')
            continue;
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 3)
            continue;
        GtdTimestamp ts = parse_ts(cols[0]);
        if (gtd_ts_is_none(ts))
            FAIL("events.csv: missing sys_time");
        const char *ann = (*cols[2] != '\0') ? cols[2] : NULL;
        CHECK_SDK(gtd_builder_add_event_marker(b, cols[1], ts, ann), "add_event_marker");
    }
    fclose(f);
}

/* One accumulator per channel; each CSV row is one sample, and the metadata
   columns repeat and are read once (on the first row for a name). */
typedef struct {
    char name[64];
    char unit[32];
    int has_unit;
    GtdOptF64 period_deg;
    char description[256];
    int has_description;
    char comp_storage[MAX_CH_COMPONENTS][32];
    const char *components[MAX_CH_COMPONENTS];
    size_t n_components;
    GtdTimestamp times[MAX_CH_SAMPLES];
    size_t n_times;
    double values[MAX_CH_SAMPLES * MAX_CH_COMPONENTS];
    size_t n_values;
} ChannelAcc;

static void load_channels(GtdFileBuilder *b, const char *base) {
    FILE *f = open_csv(base, "channels.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, f)) {
        fclose(f);
        return;
    }

    /* static to keep the ~40 KiB of accumulators off the stack. */
    static ChannelAcc channels[MAX_CHANNELS];
    size_t n_channels = 0;

    while (fgets(line, sizeof line, f)) {
        rtrim(line);
        if (*line == '\0')
            continue;
        char *cols[CSV_MAX_COLS];
        /* cols: name, unit, period_deg, description, components, time, values */
        if (split_csv(line, cols, CSV_MAX_COLS) < 7)
            continue;

        ChannelAcc *ch = NULL;
        for (size_t i = 0; i < n_channels; i++) {
            if (strcmp(channels[i].name, cols[0]) == 0) {
                ch = &channels[i];
                break;
            }
        }
        if (ch == NULL) {
            if (n_channels >= MAX_CHANNELS)
                FAIL("too many channels");
            ch = &channels[n_channels++];
            memset(ch, 0, sizeof *ch);
            snprintf(ch->name, sizeof ch->name, "%s", cols[0]);
            if (*cols[1] != '\0') {
                ch->has_unit = 1;
                snprintf(ch->unit, sizeof ch->unit, "%s", cols[1]);
            }
            ch->period_deg = parse_opt_f64(cols[2]);
            if (*cols[3] != '\0') {
                ch->has_description = 1;
                snprintf(ch->description, sizeof ch->description, "%s", cols[3]);
            }
            if (*cols[4] != '\0') {
                char *ccols[MAX_CH_COMPONENTS];
                int nc = split_delim(cols[4], ';', ccols, MAX_CH_COMPONENTS);
                for (size_t i = 0; i < (size_t)nc; i++) {
                    snprintf(ch->comp_storage[i], sizeof ch->comp_storage[i], "%s", ccols[i]);
                    ch->components[i] = ch->comp_storage[i];
                }
                ch->n_components = (size_t)nc;
            }
        }

        if (ch->n_times >= MAX_CH_SAMPLES)
            FAIL("too many channel samples");
        ch->times[ch->n_times++] = parse_ts(cols[5]);

        char *vcols[MAX_CH_COMPONENTS];
        int nv = split_delim(cols[6], ';', vcols, MAX_CH_COMPONENTS);
        for (size_t i = 0; i < (size_t)nv; i++) {
            char *end;
            double v = strtod(vcols[i], &end);
            if (end == vcols[i])
                FAIL("invalid channel value");
            if (ch->n_values >= MAX_CH_SAMPLES * MAX_CH_COMPONENTS)
                FAIL("too many channel values");
            ch->values[ch->n_values++] = v;
        }
    }
    fclose(f);

    for (size_t i = 0; i < n_channels; i++) {
        ChannelAcc *ch = &channels[i];
        GtdChannel c = {0};
        c.name = ch->name;
        c.unit = ch->has_unit ? ch->unit : NULL;
        c.period_deg = ch->period_deg;
        c.description = ch->has_description ? ch->description : NULL;
        c.components = ch->n_components ? ch->components : NULL;
        c.n_components = ch->n_components;
        c.times = ch->times;
        c.n_times = ch->n_times;
        c.values = ch->values;
        c.n_values = ch->n_values;
        CHECK_SDK(gtd_builder_add_channel(b, &c), "add_channel");
    }
}

static void verify_counts(const GtdNavFile *f) {
    const char *title = gtd_nav_file_title(f);
    const char *device = gtd_nav_file_device(f);
    const char *notes = gtd_nav_file_notes(f);
    const char *identity = gtd_nav_file_identity(f);

    CHECK(title && strstr(title, "Gold Dataset") != NULL, "title missing");
    CHECK(device && strstr(device, "Synthetic Generator") != NULL, "device missing");
    CHECK(notes && strstr(notes, "cross-SDK") != NULL, "notes missing");
    CHECK(identity && strcmp(identity, "gold-standard-v2") == 0, "identity wrong");

    size_t np = gtd_nav_file_nav_point_count(f);
    if (np != 199)
        FAILF("expected 199 nav points, got %zu", np);

    size_t anti = 0;
    for (size_t i = 0; i < np; i++) {
        GtdNavPointInfo p;
        CHECK_SDK(gtd_nav_file_get_nav_point(f, i, &p), "get_nav_point");
        if (p.lon_deg > 179.9 || p.lon_deg < -179.9)
            anti++;
    }
    if (anti != 10)
        FAILF("expected 10 antimeridian points, got %zu", anti);

    size_t em = gtd_nav_file_event_marker_count(f);
    if (em != 6)
        FAILF("expected 6 event markers, got %zu", em);
}

int main(int argc, char **argv) {
    const char *base = (argc >= 2) ? argv[1] : "tests/fixtures/gold_dataset";

    char out_path[512];
    int n = snprintf(out_path, sizeof out_path, "%s/gold_c.gtd", base);
    if (n < 0 || (size_t)n >= sizeof out_path)
        FAIL("out path too long");

    GtdFileBuilder *b = gtd_builder_create();
    gtd_builder_set_lenient(b);

    load_meta(b, base);
    load_event_styles(b, base);
    load_satellites(base);
    load_fixes(b, base);
    load_markers(b, base);
    load_events(b, base);
    load_channels(b, base);

    GtdNavFile *nav = NULL;
    GtdStatus s = gtd_builder_finish(b, &nav);
    b = NULL;
    if (s != GTD_OK)
        FAILF("finish: %s", gtd_last_error());

    CHECK_SDK(gtd_nav_file_write_to_path(nav, out_path), "write");

    verify_counts(nav);
    gtd_nav_file_destroy(nav);

    printf("Written: %s\n", out_path);
    printf("Gold dataset verified. Nav points: 189, Event markers: 6\n");
    return 0;
}
