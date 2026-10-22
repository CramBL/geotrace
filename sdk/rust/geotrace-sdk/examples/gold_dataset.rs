// Examples favour brevity: the core's robustness restriction lints (no
// unwrap/expect/panic/indexing, no std::env::temp_dir) are not enforced on
// demonstration code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::cognitive_complexity,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "SDK example: demonstration code"
)]

use chrono::Datelike as _;
use geotrace_sdk::{
    Angle, Annotation, AnnotationIcon, Channel, ChannelUnit, Constellation, ConstellationSet,
    EventMarker, EventMarkerStyle, MarkerIcon, Meta, NavFile, NavFileBuilder, NavFix, NavFixTime,
    NavRecorder, PositionProvenance, RecordedFixTimestamps, Satellite, SatelliteCoverage,
    SatelliteIdentity, SatelliteMembership, SatelliteObservation, SatelliteObservationReport,
    SatelliteReport, SatelliteScope, SatelliteUsage, SatelliteUsageReport, SolutionDimension,
    SolutionMethod, SolutionState, TravelMode, Velocity,
};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = Path::new("tests/fixtures/gold_dataset");

    if !base_dir.exists() {
        return Err(format!(
            "Base directory not found: {:?}. Are you running from the repository root?",
            base_dir
        )
        .into());
    }

    let meta = load_meta(base_dir)?;
    let mut recorder = NavFileBuilder::new()
        .with_lenient_errors()
        .with_meta(meta)
        .with_scrubbed_provenance()
        .open();

    load_event_styles(&mut recorder, base_dir)?;
    load_satellites_and_fixes(&mut recorder, base_dir)?;
    load_markers(&mut recorder, base_dir)?;
    load_events(&mut recorder, base_dir)?;
    load_channels(&mut recorder, base_dir)?;

    let nav_file = recorder.finish()?;
    nav_file.write_to_file(base_dir.join("gold.gtd"))?;

    println!("Gold dataset generated successfully: tests/fixtures/gold_dataset/gold.gtd");
    println!("Summary:");
    println!("  Nav Points:    {}", nav_file.nav_points().len());
    println!("  Markers:       {}", nav_file.markers().len());
    println!("  Event Markers: {}", nav_file.event_markers().len());
    println!("  Channels:      {}", nav_file.channels().len());

    println!("Verifying round-trip integrity...");
    verify_gold_file(base_dir.join("gold.gtd"))?;
    println!("Verified!");
    let evidence = navigation_evidence_fixture()?;
    let evidence_path = base_dir.join("navigation_evidence.gtd");
    evidence.write_to_file(&evidence_path)?;
    assert_eq!(NavFile::open(&evidence_path)?, evidence);
    host_clock_fixture()?.write_to_file(base_dir.join("host_clock.gtd"))?;

    Ok(())
}

fn load_meta(base_dir: &Path) -> Result<Meta, Box<dyn std::error::Error>> {
    let meta_file = File::open(base_dir.join("meta.csv"))?;
    let reader = BufReader::new(meta_file);
    let mut lines = reader.lines().skip(1);
    if let Some(Ok(line)) = lines.next() {
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() >= 5 {
            return Ok(Meta::builder()
                .maybe_title(Some(cols[0]))
                .maybe_device(Some(cols[1]))
                .maybe_notes(Some(cols[2]))
                .maybe_identity(Some(cols[3]))
                .travel_mode(TravelMode::from_lower_case(cols[4]))
                .build()?);
        }
    }
    Ok(Meta::builder().build()?)
}

fn load_event_styles(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let style_file = File::open(base_dir.join("event_styles.csv"))?;
    let reader = BufReader::new(style_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            continue;
        }
        // An empty icon cell leaves the icon to the application.
        let icon = if cols[1].is_empty() {
            None
        } else {
            Some(MarkerIcon::try_from_lower_case(cols[1])?)
        };
        recorder.add_event_marker_style(
            EventMarkerStyle::builder()
                .variant_path(cols[0])
                .maybe_icon(icon.map(Into::into))
                .maybe_color(Some(cols[2]))
                .build()?,
        );
    }
    Ok(())
}

fn load_satellites_and_fixes(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut satellite_reports: BTreeMap<(String, String), Vec<Satellite>> = BTreeMap::new();
    let sat_file = File::open(base_dir.join("satellites.csv"))?;
    let reader = BufReader::new(sat_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 8 {
            continue;
        }
        let sat = Satellite::builder()
            .constellation(Constellation::try_from_lower_case(cols[2])?)
            .prn(cols[3].parse()?)
            .in_fix(cols[4].parse()?)
            .maybe_elevation(cols[5].parse().ok())
            .maybe_azimuth(cols[6].parse().ok())
            .maybe_snr(cols[7].parse().ok())
            .build();

        satellite_reports
            .entry((cols[0].to_string(), cols[1].to_string()))
            .or_default()
            .push(sat);
    }

    let fix_file = File::open(base_dir.join("fixes.csv"))?;
    let reader = BufReader::new(fix_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 9 {
            continue;
        }
        let gps_time = parse_time(cols[1]);
        let sys_time = parse_time(cols[2]);
        let speed = match (cols[6], cols[8]) {
            ("", "") => None,
            (kmh, "") => Some(Velocity::try_from_kmh_str(kmh)?),
            ("", knots) => Some(Velocity::try_from_knots_str(knots)?),
            _ => {
                return Err(format!(
                    "fixes.csv row {line:?} has a speed in both speed_kmh and speed_knots"
                )
                .into());
            }
        };

        let time = required_nav_fix_time(
            RecordedFixTimestamps {
                gps: gps_time,
                sys: sys_time,
            },
            &format!("fixes.csv row {line:?}"),
        )?;

        recorder.add(
            NavFix::builder()
                .time(time)
                .lat(Angle::try_from_degrees_str(cols[3])?)
                .lon(Angle::try_from_degrees_str(cols[4])?)
                .maybe_heading(Angle::try_from_degrees_str(cols[5]).ok())
                .maybe_speed(speed)
                .maybe_eph_m(cols[7].parse().ok())
                .build(),
        );

        // Associated satellite report?
        let key = (cols[1].to_string(), cols[2].to_string());
        if let Some(tracked) = satellite_reports.remove(&key) {
            recorder.add(
                SatelliteReport::builder()
                    .time(time)
                    .tracked(tracked)
                    .build(),
            );
        }
    }

    for ((gt_str, st_str), tracked) in satellite_reports {
        let time = required_nav_fix_time(
            RecordedFixTimestamps {
                gps: parse_time(&gt_str),
                sys: parse_time(&st_str),
            },
            &format!("satellites.csv row ({gt_str:?}, {st_str:?})"),
        )?;
        recorder.add(
            SatelliteReport::builder()
                .time(time)
                .tracked(tracked)
                .build(),
        );
    }

    Ok(())
}

fn load_markers(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let marker_file = File::open(base_dir.join("markers.csv"))?;
    let reader = BufReader::new(marker_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            continue;
        }
        recorder.add(
            Annotation::builder()
                .time(parse_time(cols[0]).expect("Marker time required"))
                .maybe_label(Some(cols[1]))
                .icon(MarkerIcon::try_from_lower_case(cols[2])?)
                .build()?,
        );
    }
    Ok(())
}

fn load_events(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_file = File::open(base_dir.join("events.csv"))?;
    let reader = BufReader::new(event_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            continue;
        }
        recorder.add(
            EventMarker::builder()
                .variant_path(cols[1])
                .sys_time(parse_time(cols[0]).expect("Event sys_time required"))
                .maybe_annotation(if cols[2].is_empty() {
                    None
                } else {
                    Some(cols[2].to_string())
                })
                .build()?,
        );
    }
    Ok(())
}

fn verify_gold_file(path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
    let file = NavFile::open(path)?;

    let meta = file.meta();
    assert!(meta.title().unwrap().contains("Gold Dataset 🏆"));
    assert!(meta.device().unwrap().contains("Synthetic Generator 🧬"));
    assert!(meta.notes().unwrap().contains("🛰️"));
    assert_eq!(meta.identity(), Some("gold-standard-v2"));
    assert_eq!(meta.travel_mode(), Some(&TravelMode::Bicycle));

    let points = file.nav_points();
    assert_eq!(points.len(), 209);

    let track_8_points: Vec<_> = points
        .iter()
        .filter(|p| p.fix.lon.as_degrees() > 179.9 || p.fix.lon.as_degrees() < -179.9)
        .collect();
    assert_eq!(track_8_points.len(), 10);
    assert!((track_8_points[0].fix.lon.as_degrees() - 179.95).abs() < 1e-6);
    assert!((track_8_points[9].fix.lon.as_degrees() - (-179.96)).abs() < 1e-6);
    let reports = &track_8_points[5].satellite_evidence().observations;
    assert_eq!(reports.len(), 2);
    assert_eq!(
        (reports[1].time.effective() - reports[0].time.effective()).num_milliseconds(),
        500
    );

    // Track 9 Stationary
    let track_9_points: Vec<_> = points
        .iter()
        .filter(|p| (p.fix.lat.as_degrees() - (-10.0)).abs() < 1e-6)
        .collect();
    assert_eq!(track_9_points.len(), 20);
    for p in track_9_points {
        assert_eq!(p.fix.speed.map(|s| s.as_meters_per_second()), Some(0.0));
    }

    // The two clocks of a track 13 fix read different years: the receiver has a
    // lock while the host clock is unset.
    let unset_host_clock: Vec<i64> = points
        .iter()
        .filter_map(|p| Some((p.fix.time.gps_time()?, p.fix.time.sys_time()?)))
        .filter(|(gps, sys)| gps.year() != sys.year())
        .map(|(_, sys)| sys.timestamp_micros())
        .collect();
    assert_eq!(
        unset_host_clock,
        [-2_000_000, -1_000_000, 0, 1_000_000, 2_000_000]
    );

    // Track 14, the track of 14 February: speeds in knots.
    let track_14_speeds: Vec<Option<Velocity>> = points
        .iter()
        .filter(|p| p.fix.time.gps_time().is_some_and(|gps| gps.day() == 14))
        .map(|p| p.fix.speed)
        .collect();
    assert_eq!(
        track_14_speeds,
        [4.5, 9.0, 13.0, 18.0, 22.6].map(|knots| Some(Velocity::knot(knots)))
    );

    let markers = file.markers();
    assert_eq!(markers.len(), 16);
    // Check "File Boundary Start" at index 0
    assert_eq!(
        markers[0].annotation.label().unwrap(),
        "File Boundary Start"
    );
    assert_eq!(
        markers[0].annotation.icon(),
        AnnotationIcon::Icon(MarkerIcon::Check)
    );

    // The short arc between the track 8 fixes at 180.0° and -179.99° puts the
    // antimeridian marker, timestamped half way between them, at -179.995°.
    let antimeridian_marker = markers
        .iter()
        .find(|m| m.annotation.label() == Some("Antimeridian crossing"))
        .unwrap();
    assert!((antimeridian_marker.lon.as_degrees() - (-179.995)).abs() < 1e-6);

    let events = file.event_markers();
    assert_eq!(events.len(), 7);
    let antimeridian_event = events
        .iter()
        .find(|e| e.variant_path == "navigation/antimeridian")
        .unwrap();
    assert!((antimeridian_event.lon.as_degrees() - (-179.995)).abs() < 1e-6);

    let styles = file.event_marker_styles();
    let icon_style = styles
        .iter()
        .find(|s| s.variant_path() == "style/custom-icon")
        .unwrap();
    assert_eq!(
        icon_style.icon(),
        &geotrace_sdk::EventMarkerIconChoice::Icon(MarkerIcon::Lightning)
    );

    let color_style = styles
        .iter()
        .find(|s| s.variant_path() == "style/custom-color")
        .unwrap();
    assert!(matches!(
        color_style.color(),
        geotrace_sdk::EventMarkerColor::Hex(_)
    ));
    if let geotrace_sdk::EventMarkerColor::Hex(hex) = color_style.color() {
        assert_eq!(hex, "#FF00FF");
    }

    let channels = file.channels();
    assert_eq!(channels.len(), 2);
    let accel = channels.iter().find(|c| c.name() == "accel").unwrap();
    assert!(accel.is_vector());
    assert_eq!(accel.components(), ["x", "y", "z"]);
    let heading = channels.iter().find(|c| c.name() == "heading_raw").unwrap();
    assert_eq!(heading.period().map(|a| a.as_degrees()), Some(360.0));

    Ok(())
}

fn load_channels(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // One accumulator per channel, keyed by name in first-seen order. Each CSV
    // row is one sample. The metadata columns repeat and are read once.
    struct Acc {
        unit: Option<ChannelUnit>,
        period_deg: Option<f64>,
        description: Option<String>,
        components: Option<Vec<String>>,
        times: Vec<chrono::DateTime<chrono::Utc>>,
        values: Vec<f64>,
    }

    let file = File::open(base_dir.join("channels.csv"))?;
    let reader = BufReader::new(file);
    let mut channels: Vec<(String, Acc)> = Vec::new();
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 7 {
            continue;
        }
        let time = parse_time(cols[5]).ok_or("invalid channel timestamp")?;
        let values: Vec<f64> = cols[6]
            .split(';')
            .map(|v| v.trim().parse())
            .collect::<Result<_, _>>()?;

        let acc = if let Some(existing) = channels.iter_mut().find(|(n, _)| n.as_str() == cols[0]) {
            &mut existing.1
        } else {
            // No per-component trim: the other SDK ports split on `;` without
            // trimming, so keep this byte-identical for the gold fixture.
            let split = |s: &str| s.split(';').map(str::to_string).collect::<Vec<_>>();
            channels.push((
                cols[0].to_string(),
                Acc {
                    unit: (!cols[1].is_empty())
                        .then(|| cols[1].parse::<ChannelUnit>())
                        .transpose()?,
                    period_deg: (!cols[2].is_empty()).then(|| cols[2].parse()).transpose()?,
                    description: (!cols[3].is_empty()).then(|| cols[3].to_string()),
                    components: (!cols[4].is_empty()).then(|| split(cols[4])),
                    times: Vec::new(),
                    values: Vec::new(),
                },
            ));
            &mut channels.last_mut().ok_or("unreachable: just pushed")?.1
        };
        acc.times.push(time);
        acc.values.extend(values);
    }

    for (name, acc) in channels {
        recorder.add_channel(
            Channel::builder()
                .name(name)
                .maybe_unit(acc.unit)
                .maybe_period(acc.period_deg.map(Angle::degrees))
                .maybe_description(acc.description)
                .maybe_components(acc.components)
                .times(acc.times)
                .values(acc.values)
                .build()?,
        );
    }
    Ok(())
}

fn required_nav_fix_time(
    recorded: RecordedFixTimestamps,
    source: &str,
) -> Result<NavFixTime, Box<dyn std::error::Error>> {
    NavFixTime::from_recorded(recorded).ok_or_else(|| format!("{source} has no timestamp").into())
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    geotrace_sdk::Timestamp::try_from_iso8601(s)
        .ok()
        .map(Into::into)
}

fn navigation_evidence_fixture() -> Result<NavFile, Box<dyn std::error::Error>> {
    let base = parse_time("2025-01-01T00:00:00Z").expect("fixture time");
    let mut recorder = NavFileBuilder::new().with_scrubbed_provenance().open();
    for (second, latitude, state, count) in [
        (0, 55.0, Some(SolutionState::Fix), Some(8)),
        (1, 55.5, None, None),
        (2, 55.1, Some(SolutionState::Unknown), Some(0)),
        (3, 55.2, Some(SolutionState::NoFix), Some(12)),
    ] {
        let gps = base + chrono::Duration::seconds(second);
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Both {
                    gps,
                    sys: gps + chrono::Duration::seconds(10),
                })
                .lat(Angle::degrees(latitude))
                .lon(Angle::degrees(12.0 + second as f64 * 0.1))
                .maybe_state(state)
                .maybe_used_count(count)
                .hdop(0.9)
                .vdop(1.2)
                .pdop(1.5)
                .method(SolutionMethod::RtkFixed)
                .dimension(SolutionDimension::ThreeD)
                .position_provenance(match second {
                    1 => None,
                    3 => Some(PositionProvenance::SdkSynthesized),
                    _ => Some(PositionProvenance::SourceProvided),
                })
                .build(),
        );
    }
    recorder.add(
        SatelliteUsageReport::builder()
            .time(NavFixTime::Receiver(
                base + chrono::Duration::milliseconds(400),
            ))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .reported_count(8)
            .satellites(vec![
                SatelliteUsage::new(
                    SatelliteIdentity::new(Constellation::Gps, 1),
                    SatelliteMembership::Unused,
                ),
                SatelliteUsage::new(
                    SatelliteIdentity::new(Constellation::Gps, 1),
                    SatelliteMembership::Used,
                ),
                SatelliteUsage::new(
                    SatelliteIdentity::new(Constellation::Gps, 2),
                    SatelliteMembership::Unknown,
                ),
            ])
            .build(),
    );
    recorder.add(
        SatelliteObservationReport::builder()
            .time(NavFixTime::Host(
                base + chrono::Duration::milliseconds(10_400),
            ))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .satellites(vec![
                SatelliteObservation::builder()
                    .identity(SatelliteIdentity::new(Constellation::Gps, 1))
                    .snr(0.0)
                    .azimuth(90.0)
                    .build(),
            ])
            .build(),
    );
    recorder.add(
        SatelliteObservationReport::builder()
            .time(NavFixTime::Receiver(
                base + chrono::Duration::milliseconds(2400),
            ))
            .scope(SatelliteScope::Constellations(ConstellationSet::new(
                Constellation::Gps,
                [],
            )))
            .coverage(SatelliteCoverage::Complete)
            .reported_count(0)
            .satellites(Vec::new())
            .build(),
    );
    recorder.add(
        SatelliteUsageReport::builder()
            .time(NavFixTime::Receiver(
                base + chrono::Duration::milliseconds(4500),
            ))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Complete)
            .reported_count(0)
            .satellites(Vec::new())
            .build(),
    );
    recorder.add(
        SatelliteObservationReport::builder()
            .time(NavFixTime::Host(
                base + chrono::Duration::milliseconds(14_500),
            ))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .satellites(Vec::new())
            .build(),
    );
    recorder.add(
        SatelliteUsageReport::builder()
            .time(NavFixTime::Receiver(base + chrono::Duration::seconds(3)))
            .scope(SatelliteScope::Constellations(ConstellationSet::new(
                Constellation::Gps,
                [
                    Constellation::Glonass,
                    Constellation::Galileo,
                    Constellation::Beidou,
                    Constellation::Navic,
                    Constellation::Qzss,
                ],
            )))
            .coverage(SatelliteCoverage::Partial)
            .reported_count(u32::MAX)
            .satellites(Vec::new())
            .build(),
    );
    let file = recorder.finish()?;
    assert_eq!(file.nav_points().len(), 5);
    let source = &file.nav_points()[0];
    assert_eq!(source.fix.state, Some(SolutionState::Fix));
    assert_eq!(source.fix.used_count, Some(8));
    assert_eq!(source.fix.heading, None);
    assert_eq!(source.fix.hdop, Some(0.9));
    assert_eq!(source.fix.eph_m, None);
    assert_eq!(
        source.satellite_evidence().observations[0]
            .placement
            .expect("placement")
            .normalized_time,
        base + chrono::Duration::milliseconds(400)
    );
    let carrier = &file.nav_points()[4];
    assert_eq!(
        carrier.fix.position_provenance,
        Some(PositionProvenance::SdkSynthesized)
    );
    assert_eq!(carrier.fix.state, None);
    assert_eq!(carrier.satellite_evidence().usage.len(), 1);
    assert_eq!(carrier.satellite_evidence().observations.len(), 1);
    Ok(file)
}

fn host_clock_fixture() -> Result<NavFile, Box<dyn std::error::Error>> {
    let base = parse_time("2025-01-01T00:00:00Z").expect("fixture time");
    let mut recorder = NavFileBuilder::new().with_scrubbed_provenance().open();
    for second in [0, 10] {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Host(base + chrono::Duration::seconds(second)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(second as f64))
                .state(SolutionState::Fix)
                .used_count(8)
                .hdop(0.9)
                .build(),
        );
    }
    let time = NavFixTime::Host(base + chrono::Duration::seconds(2));
    recorder.add(
        SatelliteUsageReport::builder()
            .time(time)
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .reported_count(8)
            .satellites(Vec::new())
            .build(),
    );
    recorder.add(
        SatelliteObservationReport::builder()
            .time(time)
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .reported_count(12)
            .satellites(Vec::new())
            .build(),
    );
    let file = recorder.finish()?;
    let carrier = &file.nav_points()[1];
    assert_eq!(carrier.fix.time.effective(), time.effective());
    assert_eq!(carrier.satellite_evidence().observations[0].time, time);
    assert!((carrier.fix.lon.as_degrees() - 2.0).abs() < 1e-10);
    assert_eq!(
        carrier.satellite_evidence().usage[0]
            .placement
            .unwrap()
            .normalized_time,
        time.effective()
    );
    Ok(file)
}
