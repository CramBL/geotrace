/**
 * Gold dataset reference test for the GeoTrace C SDK.
 *
 * Reads the CSV fixtures in tests/fixtures/gold_dataset/, builds a .gtd file,
 * then verifies the round-trip.  Run from the repository root:
 *
 *   ./sdk/c/build/gold/examples/gold_dataset
 */

#include "../geotrace.h"
#include "gold_timestamp.h"

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

#define CSV_BUFSIZE       4096
#define CSV_MAX_COLS      16
#define TS_BUFSIZE        48
#define MAX_SATS          2048
#define SAT_PER_FIX       64
#define MAX_CHANNELS      8
#define MAX_CH_SAMPLES    64
#define MAX_CH_COMPONENTS 8
#define CH_NAME_BUFSIZE   64
#define CH_UNIT_BUFSIZE   32
#define CH_DESC_BUFSIZE   256
#define CH_COMP_BUFSIZE   32

static void check(int condition, const char *message) {
    if (!condition) {
        FAILF("%s", message);
    }
}

static void check_sdk_status(GtdStatus status, const char *label) {
    if (status != GTD_OK) {
        FAILF("%s: %s", label, gtd_last_error());
    }
}

/* Copies `value` into `dest`, which holds `size` bytes. Exits with an error
   when `value` does not fit. */
static void copy_field(char *dest, size_t size, const char *value) {
    int written = snprintf(dest, size, "%s", value);
    if (written < 0 || (size_t)written >= size) {
        FAILF("field too long: %s", value);
    }
}

static void rtrim(char *text) {
    size_t len = strlen(text);
    while (len > 0 && (text[len - 1] == '\r' || text[len - 1] == '\n' || text[len - 1] == ' ')) {
        text[--len] = '\0';
    }
}

static int split_delim(char *line, char delim, char *cols[], int max) {
    int count = 0;
    char *cursor = line;
    while (count < max) {
        cols[count++] = cursor;
        cursor = strchr(cursor, delim);
        if (!cursor) {
            break;
        }
        *cursor++ = '\0';
    }
    return count;
}

static int split_csv(char *line, char *cols[], int max) {
    return split_delim(line, ',', cols, max);
}

static GtdOptF64 parse_opt_f64(const char *text) {
    if (!text || *text == '\0') {
        return GTD_NONE_F64;
    }
    char *end;
    double value = strtod(text, &end);
    if (end == text) {
        return GTD_NONE_F64;
    }
    return GTD_SOME_F64(value);
}

static GtdOptF32 parse_opt_f32(const char *text) {
    GtdOptF64 value = parse_opt_f64(text);
    if (!value.present) {
        return GTD_NONE_F32;
    }
    return GTD_SOME_F32((float)value.value);
}

static GtdConstellation parse_constellation(const char *name) {
    if (strcmp(name, "gps") == 0) {
        return GTD_CONSTELLATION_GPS;
    }
    if (strcmp(name, "glonass") == 0) {
        return GTD_CONSTELLATION_GLONASS;
    }
    if (strcmp(name, "galileo") == 0) {
        return GTD_CONSTELLATION_GALILEO;
    }
    if (strcmp(name, "beidou") == 0) {
        return GTD_CONSTELLATION_BEIDOU;
    }
    FAILF("unknown constellation: %s", name);
    return GTD_CONSTELLATION_GPS; /* unreachable */
}

static GtdMarkerIcon parse_icon(const char *name) {
    if (!name || *name == '\0') {
        return GTD_ICON_AUTO;
    }
    if (strcmp(name, "pin") == 0) {
        return GTD_ICON_PIN;
    }
    if (strcmp(name, "cross") == 0) {
        return GTD_ICON_CROSS;
    }
    if (strcmp(name, "circle") == 0) {
        return GTD_ICON_CIRCLE;
    }
    if (strcmp(name, "lightning") == 0) {
        return GTD_ICON_LIGHTNING;
    }
    if (strcmp(name, "warning") == 0) {
        return GTD_ICON_WARNING;
    }
    if (strcmp(name, "error") == 0) {
        return GTD_ICON_ERROR;
    }
    if (strcmp(name, "check") == 0) {
        return GTD_ICON_CHECK;
    }
    if (strcmp(name, "satellite") == 0) {
        return GTD_ICON_SATELLITE;
    }
    if (strcmp(name, "satellite_lost") == 0) {
        return GTD_ICON_SATELLITE_LOST;
    }
    if (strcmp(name, "gear") == 0) {
        return GTD_ICON_GEAR;
    }
    if (strcmp(name, "refresh") == 0) {
        return GTD_ICON_REFRESH;
    }
    if (strcmp(name, "download") == 0) {
        return GTD_ICON_DOWNLOAD;
    }
    if (strcmp(name, "upload") == 0) {
        return GTD_ICON_UPLOAD;
    }
    if (strcmp(name, "wrench") == 0) {
        return GTD_ICON_WRENCH;
    }
    return GTD_ICON_AUTO;
}

typedef struct {
    char gps_time[TS_BUFSIZE];
    char sys_time[TS_BUFSIZE];
    GtdSatellite sat;
    int taken_by_a_fix;
} SatRow;

/* The two timestamps that match a satellite row to a fix row. */
typedef struct {
    const char *gps_time;
    const char *sys_time;
} SatelliteTimeKey;

static SatRow g_sats[MAX_SATS];
static int g_sat_count = 0;

static FILE *open_csv(const char *base, const char *name) {
    char path[512];
    int written = snprintf(path, sizeof path, "%s/%s", base, name);
    if (written < 0 || (size_t)written >= sizeof path) {
        FAIL("path too long");
    }
    FILE *file = fopen(path, "r");
    if (!file) {
        FAILF("cannot open: %s", path);
    }
    return file;
}

static void load_meta(GtdFileBuilder *builder, const char *base) {
    FILE *file = open_csv(base, "meta.csv");
    char header[CSV_BUFSIZE];
    char line[CSV_BUFSIZE];
    if (!fgets(header, sizeof header, file) || !fgets(line, sizeof line, file)) {
        (void)fclose(file);
        FAIL("meta.csv: missing data row");
    }
    (void)fclose(file);
    rtrim(line);
    char *cols[CSV_MAX_COLS];
    if (split_csv(line, cols, CSV_MAX_COLS) < 5) {
        FAIL("meta.csv: need 5 columns");
    }
    check_sdk_status(gtd_builder_set_title(builder, cols[0]), "set_title");
    check_sdk_status(gtd_builder_set_device(builder, cols[1]), "set_device");
    check_sdk_status(gtd_builder_set_notes(builder, cols[2]), "set_notes");
    check_sdk_status(gtd_builder_set_identity(builder, cols[3]), "set_identity");
    GtdTravelMode travel_mode;
    check_sdk_status(gtd_travel_mode_from_name(cols[4], &travel_mode), "travel_mode_from_name");
    check_sdk_status(gtd_builder_set_travel_mode(builder, travel_mode), "set_travel_mode");
}

static void load_event_styles(GtdFileBuilder *builder, const char *base) {
    FILE *file = open_csv(base, "event_styles.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, file)) {
        (void)fclose(file);
        return;
    }
    while (fgets(line, sizeof line, file)) {
        rtrim(line);
        if (*line == '\0') {
            continue;
        }
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 3) {
            continue;
        }
        const char *color = (*cols[2] != '\0') ? cols[2] : NULL;
        check_sdk_status(
            gtd_builder_add_event_marker_style(builder, cols[0], parse_icon(cols[1]), color),
            "add_event_marker_style");
    }
    (void)fclose(file);
}

static void load_satellites(const char *base) {
    FILE *file = open_csv(base, "satellites.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, file)) {
        (void)fclose(file);
        return;
    }
    while (fgets(line, sizeof line, file)) {
        rtrim(line);
        if (*line == '\0') {
            continue;
        }
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 8) {
            continue;
        }
        if (g_sat_count >= MAX_SATS) {
            FAIL("too many satellite rows");
        }

        SatRow *row = &g_sats[g_sat_count++];
        copy_field(row->gps_time, sizeof row->gps_time, cols[0]);
        copy_field(row->sys_time, sizeof row->sys_time, cols[1]);

        char *prn_end;
        unsigned long prn = strtoul(cols[3], &prn_end, 10);
        if (prn_end == cols[3]) {
            FAIL("invalid PRN");
        }

        row->sat.constellation = parse_constellation(cols[2]);
        row->sat.prn = (uint32_t)prn;
        row->sat.in_fix = (uint8_t)(strcmp(cols[4], "true") == 0);
        row->sat.elevation_deg = parse_opt_f32(cols[5]);
        row->sat.azimuth_deg = parse_opt_f32(cols[6]);
        row->sat.snr_dbhz = parse_opt_f32(cols[7]);
    }
    (void)fclose(file);
}

/* Copies every satellite row whose timestamps equal `key` into `out`, at most
   `max` of them. Marks every matching row taken, including the rows it did not
   copy. Returns the number copied. */
static size_t take_satellites_at(SatelliteTimeKey key, GtdSatellite *out, size_t max) {
    size_t count = 0;
    for (int i = 0; i < g_sat_count; i++) {
        if (strcmp(g_sats[i].gps_time, key.gps_time) != 0 ||
            strcmp(g_sats[i].sys_time, key.sys_time) != 0) {
            continue;
        }
        g_sats[i].taken_by_a_fix = 1;
        if (count < max) {
            out[count++] = g_sats[i].sat;
        }
    }
    return count;
}

/* cols: `track_id`, `gps_time`, `sys_time`, `lat`, `lon`, `heading_deg`,
   `speed_kmh`, `eph_m` */
static void add_fix_row(GtdFileBuilder *builder, char *cols[]) {
    GtdTimestamp gps_ts = gold_parse_timestamp(cols[1]);
    GtdTimestamp sys_ts = gold_parse_timestamp(cols[2]);

    char *end;
    double lat = strtod(cols[3], &end);
    if (end == cols[3]) {
        FAIL("invalid latitude");
    }
    double lon = strtod(cols[4], &end);
    if (end == cols[4]) {
        FAIL("invalid longitude");
    }

    GtdOptF64 hdg = parse_opt_f64(cols[5]);
    GtdOptF64 spd = GTD_NONE_F64;
    if (*cols[6] != '\0') {
        double kmh = strtod(cols[6], &end);
        /* Use the same constant-multiply as Rust's MPS_PER_KMH = 1.0/3.6.
           Direct kmh/3.6 differs by 1 ULP for some values (e.g. 23.2). */
        if (end != cols[6]) {
            spd = GTD_SOME_F64(kmh * (1.0 / 3.6));
        }
    }
    GtdOptF64 eph = parse_opt_f64(cols[7]);

    check_sdk_status(gtd_builder_add_nav_fix(builder, gps_ts, sys_ts, lat, lon, hdg, spd, eph),
                     "add_nav_fix");

    SatelliteTimeKey key = {.gps_time = cols[1], .sys_time = cols[2]};
    GtdSatellite sat_buf[SAT_PER_FIX];
    size_t sat_count = take_satellites_at(key, sat_buf, SAT_PER_FIX);
    if (sat_count > 0) {
        check_sdk_status(
            gtd_builder_add_satellite_report(builder, gps_ts, sys_ts, sat_buf, sat_count),
            "add_satellite_report");
    }
}

/* Reports at a time no fix row holds. The builder gives each one a ghost fix. */
static void add_ghost_fix_reports(GtdFileBuilder *builder) {
    for (int i = 0; i < g_sat_count; i++) {
        if (g_sats[i].taken_by_a_fix) {
            continue;
        }
        SatelliteTimeKey key = {.gps_time = g_sats[i].gps_time, .sys_time = g_sats[i].sys_time};
        GtdSatellite sat_buf[SAT_PER_FIX];
        size_t sat_count = take_satellites_at(key, sat_buf, SAT_PER_FIX);
        check_sdk_status(gtd_builder_add_satellite_report(
                             builder, gold_parse_timestamp(key.gps_time),
                             gold_parse_timestamp(key.sys_time), sat_buf, sat_count),
                         "add_satellite_report");
    }
}

static void load_fixes(GtdFileBuilder *builder, const char *base) {
    FILE *file = open_csv(base, "fixes.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, file)) {
        (void)fclose(file);
        return;
    }
    while (fgets(line, sizeof line, file)) {
        rtrim(line);
        if (*line == '\0') {
            continue;
        }
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 8) {
            continue;
        }
        add_fix_row(builder, cols);
    }
    (void)fclose(file);

    add_ghost_fix_reports(builder);
}

static void load_markers(GtdFileBuilder *builder, const char *base) {
    FILE *file = open_csv(base, "markers.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, file)) {
        (void)fclose(file);
        return;
    }
    while (fgets(line, sizeof line, file)) {
        rtrim(line);
        if (*line == '\0') {
            continue;
        }
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 3) {
            continue;
        }
        GtdTimestamp timestamp = gold_parse_timestamp(cols[0]);
        if (gtd_ts_is_none(timestamp)) {
            FAIL("markers.csv: missing timestamp");
        }
        const char *label = (*cols[1] != '\0') ? cols[1] : NULL;
        GtdMarkerIcon icon = parse_icon(cols[2]);
        if (icon == GTD_ICON_AUTO) {
            icon = GTD_ICON_PIN;
        }
        check_sdk_status(gtd_builder_add_annotation(builder, timestamp, label, icon),
                         "add_annotation");
    }
    (void)fclose(file);
}

static void load_events(GtdFileBuilder *builder, const char *base) {
    FILE *file = open_csv(base, "events.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, file)) {
        (void)fclose(file);
        return;
    }
    while (fgets(line, sizeof line, file)) {
        rtrim(line);
        if (*line == '\0') {
            continue;
        }
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 3) {
            continue;
        }
        GtdTimestamp timestamp = gold_parse_timestamp(cols[0]);
        if (gtd_ts_is_none(timestamp)) {
            FAIL("events.csv: missing sys_time");
        }
        const char *annotation = (*cols[2] != '\0') ? cols[2] : NULL;
        check_sdk_status(gtd_builder_add_event_marker(builder, cols[1], timestamp, annotation),
                         "add_event_marker");
    }
    (void)fclose(file);
}

/* One accumulator per channel. Each CSV row is one sample, and the metadata
   columns repeat and are read once (on the first row for a name). Fields are
   ordered largest-alignment first to avoid struct padding. */
typedef struct {
    GtdTimestamp times[MAX_CH_SAMPLES];
    double values[MAX_CH_SAMPLES * MAX_CH_COMPONENTS];
    GtdOptF64 period_deg;
    const char *components[MAX_CH_COMPONENTS];
    size_t n_components;
    size_t n_times;
    size_t n_values;
    int has_unit;
    int has_description;
    char name[CH_NAME_BUFSIZE];
    char unit[CH_UNIT_BUFSIZE];
    char description[CH_DESC_BUFSIZE];
    char comp_storage[MAX_CH_COMPONENTS][CH_COMP_BUFSIZE];
} ChannelAcc;

/* Returns the accumulator for the channel in column 0, appending one filled
   from the row when no earlier row listed that channel. */
static ChannelAcc *channel_for_row(ChannelAcc *channels, size_t *n_channels, char *cols[]) {
    for (size_t i = 0; i < *n_channels; i++) {
        if (strcmp(channels[i].name, cols[0]) == 0) {
            return &channels[i];
        }
    }
    if (*n_channels >= MAX_CHANNELS) {
        FAIL("too many channels");
    }

    ChannelAcc *accumulator = &channels[(*n_channels)++];
    memset(accumulator, 0, sizeof *accumulator);
    copy_field(accumulator->name, sizeof accumulator->name, cols[0]);
    if (*cols[1] != '\0') {
        accumulator->has_unit = 1;
        copy_field(accumulator->unit, sizeof accumulator->unit, cols[1]);
    }
    accumulator->period_deg = parse_opt_f64(cols[2]);
    if (*cols[3] != '\0') {
        accumulator->has_description = 1;
        copy_field(accumulator->description, sizeof accumulator->description, cols[3]);
    }
    if (*cols[4] != '\0') {
        char *component_cols[MAX_CH_COMPONENTS];
        int component_count = split_delim(cols[4], ';', component_cols, MAX_CH_COMPONENTS);
        for (size_t i = 0; i < (size_t)component_count; i++) {
            copy_field(accumulator->comp_storage[i], sizeof accumulator->comp_storage[i],
                       component_cols[i]);
            accumulator->components[i] = accumulator->comp_storage[i];
        }
        accumulator->n_components = (size_t)component_count;
    }
    return accumulator;
}

static void append_sample_values(ChannelAcc *accumulator, char *values_col) {
    char *value_cols[MAX_CH_COMPONENTS];
    int value_count = split_delim(values_col, ';', value_cols, MAX_CH_COMPONENTS);
    for (size_t i = 0; i < (size_t)value_count; i++) {
        char *end;
        double value = strtod(value_cols[i], &end);
        if (end == value_cols[i]) {
            FAIL("invalid channel value");
        }
        if (accumulator->n_values >= (size_t)MAX_CH_SAMPLES * MAX_CH_COMPONENTS) {
            FAIL("too many channel values");
        }
        accumulator->values[accumulator->n_values++] = value;
    }
}

/* cols: `name`, `unit`, `period_deg`, `description`, `components`, `time`,
   `values` */
static void load_channels(GtdFileBuilder *builder, const char *base) {
    FILE *file = open_csv(base, "channels.csv");
    char line[CSV_BUFSIZE];
    /* skip header */
    if (!fgets(line, sizeof line, file)) {
        (void)fclose(file);
        return;
    }

    /* static to keep the ~40 KiB of accumulators off the stack. */
    static ChannelAcc channels[MAX_CHANNELS];
    size_t n_channels = 0;

    while (fgets(line, sizeof line, file)) {
        rtrim(line);
        if (*line == '\0') {
            continue;
        }
        char *cols[CSV_MAX_COLS];
        if (split_csv(line, cols, CSV_MAX_COLS) < 7) {
            continue;
        }

        ChannelAcc *accumulator = channel_for_row(channels, &n_channels, cols);
        if (accumulator->n_times >= MAX_CH_SAMPLES) {
            FAIL("too many channel samples");
        }
        GtdTimestamp timestamp = gold_parse_timestamp(cols[5]);
        if (gtd_ts_is_none(timestamp)) {
            FAIL("invalid channel timestamp");
        }
        accumulator->times[accumulator->n_times++] = timestamp;

        append_sample_values(accumulator, cols[6]);
    }
    (void)fclose(file);

    for (size_t i = 0; i < n_channels; i++) {
        const ChannelAcc *accumulator = &channels[i];
        GtdChannel channel = {0};
        channel.name = accumulator->name;
        channel.unit = accumulator->has_unit ? accumulator->unit : NULL;
        channel.period_deg = accumulator->period_deg;
        channel.description = accumulator->has_description ? accumulator->description : NULL;
        channel.components = accumulator->n_components ? accumulator->components : NULL;
        channel.n_components = accumulator->n_components;
        channel.times = accumulator->times;
        channel.n_times = accumulator->n_times;
        channel.values = accumulator->values;
        channel.n_values = accumulator->n_values;
        check_sdk_status(gtd_builder_add_channel(builder, &channel), "add_channel");
    }
}

static void verify_metadata(const GtdNavFile *file) {
    const char *title = gtd_nav_file_title(file);
    const char *device = gtd_nav_file_device(file);
    const char *notes = gtd_nav_file_notes(file);
    const char *identity = gtd_nav_file_identity(file);
    const char *travel_mode = gtd_nav_file_travel_mode(file);

    check(title && strstr(title, "Gold Dataset") != NULL, "title missing");
    check(device && strstr(device, "Synthetic Generator") != NULL, "device missing");
    check(notes && strstr(notes, "cross-SDK") != NULL, "notes missing");
    check(identity && strcmp(identity, "gold-standard-v2") == 0, "identity wrong");
    check(travel_mode && strcmp(travel_mode, "bicycle") == 0, "travel mode wrong");
}

static void verify_nav_points(const GtdNavFile *file) {
    size_t nav_points = gtd_nav_file_nav_point_count(file);
    if (nav_points != 200) {
        FAILF("expected 200 nav points, got %zu", nav_points);
    }

    size_t antimeridian = 0;
    for (size_t i = 0; i < nav_points; i++) {
        GtdNavPointInfo point;
        check_sdk_status(gtd_nav_file_get_nav_point(file, i, &point), "get_nav_point");
        if (point.lon_deg > 179.9 || point.lon_deg < -179.9) {
            antimeridian++;
        }
    }
    if (antimeridian != 11) {
        FAILF("expected 11 antimeridian points, got %zu", antimeridian);
    }
}

static void verify_channels(const GtdNavFile *file) {
    size_t channel_count = gtd_nav_file_channel_count(file);
    if (channel_count != 2) {
        FAILF("expected 2 channels, got %zu", channel_count);
    }
    for (size_t i = 0; i < channel_count; i++) {
        GtdChannelInfo info;
        check_sdk_status(gtd_nav_file_get_channel(file, i, &info), "get_channel");
        if (strcmp(info.name, "accel") == 0) {
            check(info.component_count == 3, "accel should have 3 components");
        }
        if (strcmp(info.name, "heading_raw") == 0) {
            check(info.period_deg.present && info.period_deg.value == 360.0,
                  "heading_raw period wrong");
        }
    }
}

static void verify_counts(const GtdNavFile *file) {
    verify_metadata(file);
    verify_nav_points(file);

    size_t event_markers = gtd_nav_file_event_marker_count(file);
    if (event_markers != 7) {
        FAILF("expected 7 event markers, got %zu", event_markers);
    }

    verify_channels(file);
}

int main(int argc, char **argv) {
    const char *base = (argc >= 2) ? argv[1] : "tests/fixtures/gold_dataset";

    char out_path[512];
    int written = snprintf(out_path, sizeof out_path, "%s/gold_c.gtd", base);
    if (written < 0 || (size_t)written >= sizeof out_path) {
        FAIL("out path too long");
    }

    GtdFileBuilder *builder = gtd_builder_create();
    check_sdk_status(gtd_builder_set_lenient(builder), "set_lenient");

    load_meta(builder, base);
    load_event_styles(builder, base);
    load_satellites(base);
    load_fixes(builder, base);
    load_markers(builder, base);
    load_events(builder, base);
    load_channels(builder, base);

    GtdNavFile *nav = NULL;
    GtdStatus status = gtd_builder_finish(builder, &nav);
    builder = NULL;
    if (status != GTD_OK) {
        FAILF("finish: %s", gtd_last_error());
    }

    check_sdk_status(gtd_nav_file_write_to_path(nav, out_path), "write");

    verify_counts(nav);
    gtd_nav_file_destroy(nav);

    printf("Written: %s\n", out_path);
    printf("Gold dataset verified. Nav points: 200, Event markers: 7, Channels: 2\n");
    return 0;
}
