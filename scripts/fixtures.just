# Fixture capture and verification recipes. Fixtures are frozen once
# committed - review the resulting diff like code.

# Generate the minimal.gtd fixture used by the C SDK tests.
[group("fixtures")]
gen-fixture:
    cargo run -p geotrace-c --bin gen_fixture

# Refresh the gt-flare event fixtures from the NASA DONKI catalog (network!).
# Needs GEOTRACE_FLARE_API_KEY, an api.nasa.gov key.
# Name windows to capture only those (additive); no args captures all.
[group("fixtures")]
flare-fixtures *WINDOWS:
    cargo run -p gt-flare --example fetch_flare_fixtures -- {{ WINDOWS }}

# Refresh the gt-ionex map fixtures from the JPL archive (network!).
# Name files to capture only those (additive); no args captures all.
[group("fixtures")]
ionex-fixtures *FILES:
    cargo run -p gt-ionex --example fetch_ionex_fixtures -- {{ FILES }}

# Check the CDDIS mirror addressing against the live archive (network!).
# Needs EARTHDATA_TOKEN, a NASA Earthdata token.
# Pass --day YYYY-MM-DD to address another day than 2024-05-10, such as one
# from the years the archive files under the legacy short names.
# Pass --capture to write the served files and their manifest entries under
# gt-ionex/tests/fixtures/cddis/.
[group("fixtures")]
cddis-verify *ARGS:
    cargo run -p gt-ionex --example verify_cddis_mirror -- {{ ARGS }}

# Refresh the gt-ionex node-series fixture from the JPL archive (network!).
# Captures the storm days' per-node values the quiet-time grading reads.
[group("fixtures")]
ionex-node-series:
    cargo run -p gt-ionex --example fetch_node_series_fixture

# Refresh the gt-jam interference dataset fixtures from gpsjam.org (network!).
# Name days to capture only those (additive); no args captures all.
[group("fixtures")]
jam-fixtures *DAYS:
    cargo run -p gt-jam --example fetch_jam_fixtures -- {{ DAYS }}

# Capture the Mapbox satellite tiles the fixture-backed map snapshots draw
# (network!). Needs MAPBOX_TOKEN.
# Name files a snapshot run recorded under GEOTRACE_RECORD_TILE_MISSES to add
# their tiles (additive); no args re-captures what the manifest already lists.
[group("fixtures")]
map-tile-fixtures *WANTED:
    cargo run -p gt-map --example fetch_map_tile_fixtures -- {{ WANTED }}

# Refresh the gt-solar index fixtures from the GFZ Potsdam service (network!).
# Name windows to capture only those (additive); no args captures all.
[group("fixtures")]
solar-fixtures *WINDOWS:
    cargo run -p gt-solar --example fetch_solar_fixtures -- {{ WINDOWS }}

# Refresh the gt-snap live-API fixtures from the Valhalla server (network!).
# Name scenarios to capture only those (additive); no args captures all.
[group("fixtures")]
snap-fixtures *SCENARIOS:
    cargo run -p gt-snap --example fetch_snap_fixtures -- {{ SCENARIOS }}
