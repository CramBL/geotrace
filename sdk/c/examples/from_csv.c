/**
 * Convert GPS data from CSV text into a .gtd GeoTrace data file.
 *
 * Scenario: your GPS logger exports fixes as CSV rows.  Parse each row, feed
 * the fields to the builder, then finish() to produce a validated file ready
 * for GeoTrace to open.  In a real workflow you would read the CSV from a file.
 *
 * Timestamps here are whole Unix epoch seconds to keep the parser tiny. A
 * logger emitting ISO-8601 strings would parse them as in gold_dataset.c.
 */

#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *const CSV_DATA = "timestamp_s,lat,lon,heading_deg,speed_mps\n"
                                    "1705309200,51.5074,-0.1278,90.0,12.5\n"
                                    "1705309201,51.5075,-0.1276,91.0,12.6\n"
                                    "1705309202,51.5076,-0.1274,89.5,12.4\n"
                                    "1705309203,51.5077,-0.1272,88.0,12.3\n"
                                    "1705309204,51.5078,-0.1270,90.0,12.5\n"
                                    "1705309205,51.5079,-0.1268,90.5,12.6\n";

#define LINE_BUFSIZE 128
#define CSV_COLS     5

/* Parse "ts,lat,lon,heading,speed" into out[CSV_COLS]. The first column is an
   integer (seconds), the rest are doubles. Returns 1 on success, 0 on a
   malformed row. A row is malformed when `strtod` or `strtoull` leaves the
   end pointer at the start of a field. */
static int parse_row(const char *line, unsigned long long *ts, double out[CSV_COLS - 1]) {
    char *end;
    *ts = strtoull(line, &end, 10);
    if (end == line || *end != ',')
        return 0;

    const char *cur = end + 1;
    for (int i = 0; i < CSV_COLS - 1; i++) {
        out[i] = strtod(cur, &end);
        if (end == cur)
            return 0;
        /* Every field but the last must be followed by a comma. */
        if (i < CSV_COLS - 2) {
            if (*end != ',')
                return 0;
            cur = end + 1;
        }
    }
    return 1;
}

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();

    gtd_builder_set_title(b, "Imported from CSV");
    gtd_builder_set_device(b, "CSV importer v1.0");

    const char *p = CSV_DATA;
    /* Skip the header row. */
    p = strchr(p, '\n');
    if (p)
        p++;

    size_t rows = 0;
    while (p && *p != '\0') {
        const char *nl = strchr(p, '\n');
        size_t len = nl ? (size_t)(nl - p) : strlen(p);

        if (len > 0 && len < LINE_BUFSIZE) {
            char line[LINE_BUFSIZE];
            memcpy(line, p, len);
            line[len] = '\0';

            unsigned long long ts = 0;
            double v[CSV_COLS - 1];
            if (parse_row(line, &ts, v)) {
                GtdStatus s = gtd_builder_add_nav_fix(b, gtd_ts_from_seconds((uint64_t)ts),
                                                      gtd_ts_none(), v[0], v[1], GTD_SOME_F64(v[2]),
                                                      GTD_SOME_F64(v[3]), GTD_NONE_F64);
                if (s != GTD_OK) {
                    fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
                    gtd_builder_destroy(b);
                    return 1;
                }
                rows++;
            }
        }

        p = nl ? nl + 1 : NULL;
    }

    GtdNavFile *f = NULL;
    GtdStatus s = gtd_builder_finish(b, &f);
    b = NULL;
    if (s != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    const char *path = "geotrace_from_csv.gtd";
    s = gtd_nav_file_write_to_path(f, path);
    if (s != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        gtd_nav_file_destroy(f);
        return 1;
    }

    printf("Parsed %zu CSV rows into %zu nav points -> %s\n", rows, gtd_nav_file_nav_point_count(f),
           path);

    gtd_nav_file_destroy(f);
    remove(path);
    return 0;
}
