# GeoTrace Gold Dataset

This dataset provides a set of reference GPS/GNSS data in CSV format.
It is intended for cross-SDK verification: every SDK (Rust, C, C++, Python) parses these CSV files and writes a `.gtd` file that decodes to the same `NavFile`.
The HDF5 byte layout may differ between SDKs.
The decoded content must not.
The `gold_conformance` test (`sdk/rust/geotrace-sdk/tests/gold_conformance.rs`) pins `gold.gtd`, `gold_c.gtd`, `gold_cpp.gtd`, and `gold_py.gtd` to this guarantee, and `just test-gold-all` regenerates and re-checks them.

## Dataset Structure

- `meta.csv`: The recording's title, device, notes, identity and travel mode.
- `fixes.csv`: Primary navigation data (TPV). 204 fixes over 13 tracks.
- `satellites.csv`: Satellite visibility reports associated with the fixes.
- `markers.csv`: User-defined map annotations (Markers). 16 markers covering peaks, starts, sub-second interpolation and the antimeridian crossing.
- `events.csv`: System event markers. 7 events for status changes, turns, signal loss and the antimeridian crossing.
- `event_styles.csv`: Icon and color overrides for two of the event variants.
- `channels.csv`: Ad-hoc sensor channels correlated with the track by timestamp. One row per sample. The metadata columns (`unit`, `period_deg`, `description`, `components`) repeat per channel, and `components`/`values` are `;`-separated. Covers a vector channel (`accel`, x/y/z, unit g) and a scalar channel with a wrap period (`heading_raw`, deg, period 360).

## Track Definitions

Each track starts one day after the one before it, the first on **1 February 2026 at 15:00:00 UTC**.
Tracks 1 to 7, track 12 and track 13 start in the Sahara desert, offset from 23.0°N, 13.0°E to sit apart on the map.
Tracks 1 to 5 are offset by up to 0.05° on each axis.
Track 6 is offset by 0.1° and track 7 by 0.2°, both on latitude and longitude.
Track 12 is offset by 0.3° of latitude and track 13 by 0.4° of latitude.
Tracks 8 to 11 start at the coordinates named in their own sections.

### Track 1: Straight Line (North)
- **Purpose**: Baseline linear movement and speed acceleration.
- **Description**: 10 points spaced 2 meters apart, moving North.
- **Speed**: Starts at 0 km/h and increases by 10 km/h at each point (up to 90 km/h).
- **Satellites**: 49 per fix across GPS, GLONASS, Galileo and BeiDou, 25 of them in the fix.

### Track 2: Sine Curve
- **Purpose**: Periodic lateral movement.
- **Description**: 20 points moving North with a sine wave oscillation on Longitude (10m amplitude).
- **Satellites**: Multi-constellation (GPS + GLONASS).

### Track 3: Spiral
- **Purpose**: Complex angular movement and increasing radius.
- **Description**: 20 points forming a spiral, 45° turn and 1m radius increase per point.
- **Verification**: Includes a marker with a sub-second time offset (10.5s) to test linear interpolation logic.

### Track 4: L-Shape (90° Turn)
- **Purpose**: Sharp cornering.
- **Description**: 10 meters East, followed by a sharp 90° turn and 10 meters South.

### Track 5: Wide Arc (Soft Turn)
- **Purpose**: Gradual heading change.
- **Description**: 50 meters West, followed by a soft 30° turn towards North over 10 seconds.

### Track 6: Point Cluster
- **Purpose**: Testing precision and rendering of closely packed points.
- **Description**: 10 points spaced 1cm apart, 9cm end to end.

### Track 7: Fix Loss / Gain
- **Purpose**: Testing SDK behavior during no-fix periods.
- **Description**: 10 points moving North. GPS lock is lost between points 3 and 7.
- **Details**: Satellite reports continue during the no-fix period. Includes event markers for "signal lost" and "signal regained", and a marker placed during the no-fix period to verify interpolation between ghost fixes.

### Track 8: Antimeridian Crossing
- **Purpose**: Verify coordinate wrapping and interpolation.
- **Description**: Moves from Longitude 179.95° to -179.96° across the 180° meridian.
- **Verification**: A marker, an event marker and an orphan satellite report share the time 15:00:05.5, half way between the fixes at 180.0° and -179.99°.
  The SDK places all three at -179.995°, on the short arc between those two fixes.

### Track 9: Stationary (Zero Speed)
- **Purpose**: Stress test for zero-delta movement.
- **Description**: 20 points at exactly the same location (10.0°S, 10.0°W) with zero speed.

### Track 10: Satellite Stress
- **Purpose**: Trigger validation warnings.
- **Description**: 5 points at 45.0°N, 45.0°E. Every fix reports PRN 0, an SNR of 99 (the no-data reading), a repeated GPS PRN 1, a GLONASS PRN of 70 and a BeiDou PRN of 100.

### Track 11: Metadata & Unicode Stress
- **Purpose**: Verify string handling and serialization.
- **Description**: 2 points at 10.0°N, 10.0°E. The metadata fields (title, notes) contain long strings and Unicode emojis.

### Track 12: Fix Acquired Mid-Track
- **Purpose**: Rendering a track that starts without a receiver fix and gains one part-way through.
- **Description**: 10 points moving North. The first 3 have a `sys_time` only, the remaining 7 have a `gps_time`, a `sys_time` and an `eph_m`.

### Track 13: Host Clock Before the Unix Epoch
- **Purpose**: Verify that every SDK writes and reads a timestamp before 1970.
- **Description**: 5 points moving North.
  Each fix has a `gps_time` of 13 February 2026 from the receiver's lock and a `sys_time` from an unset real-time clock.
  Two of the five `sys_time` values fall in 1969, before the Unix epoch, and three fall in 1970.
- **Verification**: The five `sys_time` values run from `1969-12-31T23:59:58+00:00` to `1970-01-01T00:00:02+00:00`.
  The microsecond counts the writer stores for them are negative, zero and positive.

## Event Styling

The dataset includes explicit style overrides for certain event variants:
- `style/custom-icon`: Overridden with the `Lightning` icon.
- `style/custom-color`: Overridden with `#FF00FF` (Magenta).
