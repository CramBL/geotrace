# GeoTrace Gold Dataset

This dataset provides a set of reference GPS/GNSS data in CSV format.
It is intended for cross-SDK verification: every SDK (Rust, C, C++, Python) parses these CSV files and writes a `.gtd` file that decodes to the same `NavFile`.
The HDF5 byte layout may differ between SDKs; the decoded content must not.
The `gold_conformance` test (`sdk/rust/geotrace-sdk/tests/gold_conformance.rs`) pins `gold.gtd`, `gold_c.gtd`, `gold_cpp.gtd`, and `gold_py.gtd` to this guarantee, and `just test-gold-all` regenerates and re-checks them.

## Dataset Structure

- `fixes.csv`: Primary navigation data (TPV).
- `satellites.csv`: Satellite visibility reports associated with the fixes.
- `markers.csv`: User-defined map annotations (Markers). Now includes 15 markers covering peaks, starts, and sub-second interpolation.
- `events.csv`: System event markers. Now includes 5 events for status changes, turns, and signal loss.

## Track Definitions

All tracks start in the Sahara desert (23.0° N, 13.0° E) and are separated by exactly 1 day.
The first track starts on **1 February 2026 at 15:00:00 UTC**.

### Track 1: Straight Line (North)
- **Purpose**: Baseline linear movement and speed acceleration.
- **Description**: 10 points spaced 2 meters apart, moving North.
- **Speed**: Starts at 0 km/h and increases by 10 km/h at each point (up to 90 km/h).
- **Satellites**: GPS only (PRNs 1-4).

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
- **Description**: 10 points clustered within a 1cm radius.

### Track 7: Fix Loss / Gain
- **Purpose**: Testing SDK behavior during no-fix periods.
- **Description**: 10 points moving North. GPS lock is lost between points 3 and 7.
- **Details**: Satellite reports continue during the no-fix period. Includes event markers for "signal lost" and "signal regained", and a marker placed during the no-fix period to verify interpolation between ghost fixes.

### Track 8: Antimeridian Crossing
- **Purpose**: Verify coordinate wrapping and interpolation.
- **Description**: Moves from Longitude 179.95° to -179.95° across the 180° meridian.

### Track 9: Stationary (Zero Speed)
- **Purpose**: Stress test for zero-delta movement.
- **Description**: 20 points at exactly the same location with zero speed.

### Track 10: Satellite Stress
- **Purpose**: Trigger validation warnings.
- **Description**: Includes PRN 0, SNR 99 (sentinel), duplicate PRNs, and out-of-range PRNs (SBAS/Glonass offsets).

### Track 11: Metadata & Unicode Stress
- **Purpose**: Verify string handling and serialization.
- **Description**: Metadata fields (title, notes) contain long strings and Unicode emojis.

## Event Styling

The dataset includes explicit style overrides for certain event variants:
- `style/custom-icon`: Overridden with the `Lightning` icon.
- `style/custom-color`: Overridden with `#FF00FF` (Magenta).
