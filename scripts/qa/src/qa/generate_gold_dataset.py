import csv
import math
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

BASE_TIME = datetime(2026, 2, 1, 15, 0, 0, tzinfo=UTC)
SAHARA_LAT = 23.0
SAHARA_LON = 13.0

# Constants for coordinate calculations
METERS_PER_DEGREE_LAT = 111132.0
METERS_PER_DEGREE_LON = METERS_PER_DEGREE_LAT * math.cos(math.radians(SAHARA_LAT))


def get_time(track_idx: int, seconds_offset: float) -> str:
    return (BASE_TIME + timedelta(days=track_idx, seconds=seconds_offset)).isoformat()


def add_meters(lat: float, lon: float, d_north: float, d_east: float) -> tuple[float, float]:
    new_lat = lat + (d_north / METERS_PER_DEGREE_LAT)
    new_lon = lon + (d_east / METERS_PER_DEGREE_LON)
    return new_lat, new_lon


def write_csv(dest_path: Path, data: list[dict[str, Any]], fieldnames: list[str]) -> None:
    with dest_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(data)


def main() -> None:
    # Find the repository root (assuming we are in scripts/qa/src/qa/generate_gold_dataset.py)
    repo_root = Path(__file__).parent.parent.parent.parent.parent
    dest_dir = repo_root / "tests" / "fixtures" / "gold_dataset"
    dest_dir.mkdir(parents=True, exist_ok=True)

    fixes = []
    satellites = []
    markers = []
    events = []

    # Track 1: Straight line moving North
    # 10 points, 2m apart, speed 0 to 90 km/h (+10 each)
    for i in range(10):
        t = get_time(0, float(i))
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, i * 2, 0)
        speed = i * 10
        fixes.append(
            {
                "track_id": 1,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 0.0,
                "speed_kmh": speed,
                "eph_m": 2.5,
            }
        )
        # Add some satellites for Track 1 (GPS only)
        for prn in range(1, 5):
            satellites.append(
                {
                    "gps_time": t,
                    "sys_time": t,
                    "constellation": "gps",
                    "prn": prn,
                    "in_fix": "true",
                    "elevation": 45,
                    "azimuth": 90 + prn * 10,
                    "snr": 35,
                }
            )

    # Track 2: Sine curve moving North
    # 20 points, sine wave on Lon
    for i in range(20):
        t = get_time(1, float(i))
        d_north_float = float(i * 5)
        d_east = 10 * math.sin(i * math.pi / 5)  # 10m amplitude, period 10 points
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, d_north_float, d_east)
        fixes.append(
            {
                "track_id": 2,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 0.0,  # heading simplified
                "speed_kmh": 18.0,
                "eph_m": 3.0,
            }
        )
        # Multi-constellation for Track 2
        for prn in range(1, 3):
            satellites.append(
                {
                    "gps_time": t,
                    "sys_time": t,
                    "constellation": "gps" if prn == 1 else "glonass",
                    "prn": prn,
                    "in_fix": "true",
                    "elevation": 30,
                    "azimuth": 45,
                    "snr": 40,
                }
            )

    # Track 3: Spiral
    # 20 points
    for i in range(20):
        t = get_time(2, float(i))
        angle = i * math.pi / 4  # 45 degrees per point
        radius = float(i * 1)  # 1m increase per point
        d_north = radius * math.cos(angle)
        d_east = radius * math.sin(angle)
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, d_north, d_east)
        fixes.append(
            {
                "track_id": 3,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": math.degrees(angle),
                "speed_kmh": 5.0,
                "eph_m": 1.5,
            }
        )

    # Track 4: L-shape
    # 10m East, then 10m South
    # Points 0-10 move East
    for i in range(11):
        t = get_time(3, float(i))
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, 0, float(i))
        fixes.append(
            {
                "track_id": 4,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 90.0,
                "speed_kmh": 3.6,
                "eph_m": 1.0,
            }
        )
    # Points 11-20 move South from (SAHARA_LAT, SAHARA_LON + 10m)
    for i in range(1, 11):
        t = get_time(3, float(10 + i))
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, float(-i), 10.0)
        fixes.append(
            {
                "track_id": 4,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 180.0,
                "speed_kmh": 3.6,
                "eph_m": 1.0,
            }
        )

    # Track 5: Soft turn
    # 50m West, then 30 degree turn North over 10s
    # Move West for 50s at 1m/s
    for i in range(51):
        t = get_time(4, float(i))
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, 0, float(-i))
        fixes.append(
            {
                "track_id": 5,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 270.0,
                "speed_kmh": 3.6,
                "eph_m": 1.0,
            }
        )
    # Turn North: 30 degrees over 10s. Final heading 300 deg.
    for i in range(1, 11):
        t = get_time(4, float(50 + i))
        angle_deg = 270 + (i * 3)  # 3 degrees per second
        # For simplicity, we just calculate the arc
        angle_rad = math.radians(angle_deg)  # noqa: F841
        d_north = float(i) * math.sin(math.radians(i * 3))  # increasing north component
        d_east = -50 - i * math.cos(math.radians(i * 3))  # still moving west-ish
        lat, lon = add_meters(SAHARA_LAT, SAHARA_LON, d_north, d_east)
        fixes.append(
            {
                "track_id": 5,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": float(angle_deg),
                "speed_kmh": 3.6,
                "eph_m": 1.0,
            }
        )

    # Add Edge Case: Cluster
    # Track 6: Clustered points
    for i in range(10):
        t = get_time(5, float(i))
        # points within 1cm
        lat, lon = add_meters(SAHARA_LAT + 0.1, SAHARA_LON + 0.1, i * 0.01, 0)
        fixes.append(
            {
                "track_id": 6,
                "gps_time": t,
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 0.0,
                "speed_kmh": 0.036,
                "eph_m": 0.5,
            }
        )

    # Add Edge Case: No fix
    # Track 7: Fix loss
    for i in range(10):
        t = get_time(6, float(i))
        lat, lon = add_meters(SAHARA_LAT + 0.2, SAHARA_LON + 0.2, i * 5, 0)
        has_fix = i < 3 or i > 7
        fixes.append(
            {
                "track_id": 7,
                "gps_time": t if has_fix else "",
                "sys_time": t,
                "lat": lat,
                "lon": lon,
                "heading_deg": 0.0,
                "speed_kmh": 18.0,
                "eph_m": 2.0 if has_fix else "",
            }
        )
        # Satellites continue even without fix
        satellites.append(
            {
                "gps_time": t if has_fix else "",
                "sys_time": t,
                "constellation": "galileo",
                "prn": 10,
                "in_fix": "true" if has_fix else "false",
                "elevation": 60,
                "azimuth": 180,
                "snr": 25,
            }
        )

    # Track 8: Antimeridian Crossing
    # Move from 179.99 to -179.99
    for i in range(10):
        t = get_time(7, float(i))
        lon = 179.95 + (i * 0.01)
        if lon > 180:
            lon -= 360
        fixes.append(
            {
                "track_id": 8,
                "gps_time": t,
                "sys_time": t,
                "lat": 0.0,
                "lon": lon,
                "heading_deg": 90.0,
                "speed_kmh": 36.0,
                "eph_m": 1.0,
            }
        )

    # Track 9: Stationary (Zero Speed)
    # 20 points at exact same location
    for i in range(20):
        t = get_time(8, float(i))
        fixes.append(
            {
                "track_id": 9,
                "gps_time": t,
                "sys_time": t,
                "lat": -10.0,
                "lon": -10.0,
                "heading_deg": 0.0,
                "speed_kmh": 0.0,
                "eph_m": 0.5,
            }
        )

    # Track 10: Satellite Stress (Triggering Warnings)
    for i in range(5):
        t = get_time(9, float(i))
        fixes.append(
            {
                "track_id": 10,
                "gps_time": t,
                "sys_time": t,
                "lat": 45.0,
                "lon": 45.0,
                "heading_deg": 0.0,
                "speed_kmh": 10.0,
                "eph_m": 1.0,
            }
        )
        # Add problematic satellites
        sats_to_add = [
            {"constellation": "gps", "prn": 0, "snr": 30},  # Invalid PRN
            {"constellation": "gps", "prn": 1, "snr": 99},  # SNR Sentinel
            {"constellation": "glonass", "prn": 70, "snr": 35},  # Glonass Offset
            {"constellation": "gps", "prn": 1, "snr": 40},  # Duplicate PRN
            {"constellation": "beidou", "prn": 100, "snr": 20},  # Out of range
        ]
        for s in sats_to_add:
            satellites.append(
                {
                    "gps_time": t,
                    "sys_time": t,
                    "constellation": s["constellation"],
                    "prn": s["prn"],
                    "in_fix": "true",
                    "elevation": 45,
                    "azimuth": 0,
                    "snr": s["snr"],
                }
            )

    # Track 11: Unicode and Metadata Stress
    # Just 2 points
    for i in range(2):
        t = get_time(10, float(i))
        fixes.append(
            {
                "track_id": 11,
                "gps_time": t,
                "sys_time": t,
                "lat": 10.0,
                "lon": 10.0,
                "heading_deg": 0.0,
                "speed_kmh": 0.0,
                "eph_m": 0.1,
            }
        )

    # Add some markers and events
    # Boundary markers (Exactly at start/end of Track 1)
    markers.append({"time": get_time(0, 0), "label": "File Boundary Start", "icon": "check"})
    markers.append({"time": get_time(0, 9), "label": "File Boundary End", "icon": "cross"})

    # Event Style Overrides
    events.append(
        {
            "sys_time": get_time(1, 0),
            "variant_path": "style/custom-icon",
            "annotation": "This should have a custom icon",
        }
    )
    events.append(
        {
            "sys_time": get_time(1, 1),
            "variant_path": "style/custom-color",
            "annotation": "This should have a custom color",
        }
    )

    # Track 2: Sine curve peaks
    for i in [2, 7, 12, 17]:
        markers.append(
            {"time": get_time(1, float(i)), "label": f"Peak {i // 5 + 1}", "icon": "lightning"}
        )

    # Track 3: Spiral
    # Test interpolation: marker at 0.5s offset
    markers.append(
        {
            "time": (BASE_TIME + timedelta(days=2, seconds=10, milliseconds=500)).isoformat(),
            "label": "Interpolated spiral point",
            "icon": "cross",
        }
    )
    events.append(
        {"sys_time": get_time(2, 19), "variant_path": "spiral/end", "annotation": "Spiral complete"}
    )

    # Track 4: L-shape
    events.append(
        {
            "sys_time": get_time(3, 10),
            "variant_path": "navigation/turn-90",
            "annotation": "Sharp right turn",
        }
    )

    # Track 5: Soft turn
    # Multiple markers along the turn
    for i in range(50, 61, 2):
        markers.append(
            {"time": get_time(4, float(i)), "label": f"Turn point {i}", "icon": "refresh"}
        )

    # Track 6: Cluster
    markers.append({"time": get_time(5, 5), "label": "Cluster center", "icon": "wrench"})

    # Track 7: Fix loss
    events.append(
        {"sys_time": get_time(6, 3), "variant_path": "signal/lost", "annotation": "GNSS fix lost"}
    )
    events.append(
        {
            "sys_time": get_time(6, 8),
            "variant_path": "signal/regained",
            "annotation": "GNSS fix regained",
        }
    )
    # Marker during no-fix period (interpolated between ghosts)
    markers.append(
        {"time": get_time(6, 5), "label": "Marker during no-fix", "icon": "satellite_lost"}
    )

    # Metadata
    meta = [
        {
            "title": "Gold Dataset 🏆 — Unicode & Long Strings Test: " + "A" * 100,
            "device": "Synthetic Generator 🧬",
            "notes": "Standard test dataset for cross-SDK verification. "
            "Includes emoji 🛰️ and long notes: " + "B" * 200,
            "identity": "gold-standard-v2",
        }
    ]

    # Event Styles
    event_styles = [
        {"variant_path": "style/custom-icon", "icon": "lightning", "color": ""},
        {"variant_path": "style/custom-color", "icon": "", "color": "#FF00FF"},
    ]

    write_csv(
        dest_dir / "fixes.csv",
        fixes,
        ["track_id", "gps_time", "sys_time", "lat", "lon", "heading_deg", "speed_kmh", "eph_m"],
    )
    write_csv(
        dest_dir / "satellites.csv",
        satellites,
        ["gps_time", "sys_time", "constellation", "prn", "in_fix", "elevation", "azimuth", "snr"],
    )
    write_csv(dest_dir / "markers.csv", markers, ["time", "label", "icon"])
    write_csv(dest_dir / "events.csv", events, ["sys_time", "variant_path", "annotation"])
    write_csv(dest_dir / "meta.csv", meta, ["title", "device", "notes", "identity"])
    write_csv(dest_dir / "event_styles.csv", event_styles, ["variant_path", "icon", "color"])


if __name__ == "__main__":
    main()
