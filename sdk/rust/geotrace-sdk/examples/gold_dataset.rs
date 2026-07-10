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

use geotrace_sdk::{
    Angle, Annotation, Channel, ChannelUnit, Constellation, EventMarker, EventMarkerStyle,
    MarkerIcon, Meta, NavFileBuilder, NavFix, NavRecorder, Satellite, SatelliteReport, Velocity,
};
use std::collections::HashMap;
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

    Ok(())
}

fn load_meta(base_dir: &Path) -> Result<Meta, Box<dyn std::error::Error>> {
    let meta_file = File::open(base_dir.join("meta.csv"))?;
    let reader = BufReader::new(meta_file);
    let mut lines = reader.lines().skip(1);
    if let Some(Ok(line)) = lines.next() {
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() >= 4 {
            return Ok(Meta::builder()
                .maybe_title(Some(cols[0]))
                .maybe_device(Some(cols[1]))
                .maybe_notes(Some(cols[2]))
                .maybe_identity(Some(cols[3]))
                .build());
        }
    }
    Ok(Meta::builder().build())
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
        recorder.add_event_marker_style(
            EventMarkerStyle::builder()
                .variant_path(cols[0])
                .maybe_icon(
                    MarkerIcon::try_from_lower_case(cols[1])
                        .ok()
                        .map(Into::into),
                )
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
    let mut satellite_reports: HashMap<(String, String), Vec<Satellite>> = HashMap::new();
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
        if cols.len() < 8 {
            continue;
        }
        let gps_time = parse_time(cols[1]);
        let sys_time = parse_time(cols[2]);

        recorder.add(
            NavFix::builder()
                .maybe_gps_time(gps_time)
                .maybe_sys_time(sys_time)
                .lat(Angle::try_from_degrees_str(cols[3])?)
                .lon(Angle::try_from_degrees_str(cols[4])?)
                .maybe_heading(Angle::try_from_degrees_str(cols[5]).ok())
                .maybe_speed(Velocity::try_from_kmh_str(cols[6]).ok())
                .maybe_eph_m(cols[7].parse().ok())
                .build(),
        );

        // Associated satellite report?
        let key = (cols[1].to_string(), cols[2].to_string());
        if let Some(tracked) = satellite_reports.remove(&key) {
            recorder.add(
                SatelliteReport::builder()
                    .maybe_gps_time(gps_time)
                    .maybe_sys_time(sys_time)
                    .tracked(tracked)
                    .build(),
            );
        }
    }

    for ((gt_str, st_str), tracked) in satellite_reports {
        recorder.add(
            SatelliteReport::builder()
                .maybe_gps_time(parse_time(&gt_str))
                .maybe_sys_time(parse_time(&st_str))
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
                .maybe_icon(MarkerIcon::try_from_lower_case(cols[2]).ok())
                .build(),
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
    let file = geotrace_sdk::NavFile::open(path)?;

    let meta = file.meta();
    assert!(meta.title.as_ref().unwrap().contains("Gold Dataset 🏆"));
    assert!(
        meta.device
            .as_ref()
            .unwrap()
            .contains("Synthetic Generator 🧬")
    );
    assert!(meta.notes.as_ref().unwrap().contains("🛰️"));
    assert_eq!(meta.identity.as_ref().unwrap(), "gold-standard-v2");

    let points = file.nav_points();
    assert_eq!(points.len(), 199);

    // Track 8 Antimeridian: check first and last point
    let track_8_points: Vec<_> = points
        .iter()
        .filter(|p| p.fix.lon.as_degrees() > 179.9 || p.fix.lon.as_degrees() < -179.9)
        .collect();
    assert_eq!(track_8_points.len(), 10);
    assert!((track_8_points[0].fix.lon.as_degrees() - 179.95).abs() < 1e-6);
    assert!((track_8_points[9].fix.lon.as_degrees() - (-179.96)).abs() < 1e-6);

    // Track 9 Stationary
    let track_9_points: Vec<_> = points
        .iter()
        .filter(|p| (p.fix.lat.as_degrees() - (-10.0)).abs() < 1e-6)
        .collect();
    assert_eq!(track_9_points.len(), 20);
    for p in track_9_points {
        assert_eq!(p.fix.speed.map(|s| s.as_meters_per_second()), Some(0.0));
    }

    let markers = file.markers();
    assert_eq!(markers.len(), 15);
    // Check "File Boundary Start" at index 0
    assert_eq!(
        markers[0].annotation.label.as_ref().unwrap(),
        "File Boundary Start"
    );
    assert_eq!(markers[0].annotation.icon, Some(MarkerIcon::Check));

    let events = file.event_markers();
    assert_eq!(events.len(), 6);

    let styles = file.event_marker_styles();
    let icon_style = styles
        .iter()
        .find(|s| s.variant_path == "style/custom-icon")
        .unwrap();
    assert_eq!(
        icon_style.icon,
        geotrace_sdk::EventMarkerIconChoice::Icon(MarkerIcon::Lightning)
    );

    let color_style = styles
        .iter()
        .find(|s| s.variant_path == "style/custom-color")
        .unwrap();
    assert!(matches!(
        color_style.color,
        geotrace_sdk::EventMarkerColor::Hex(_)
    ));
    if let geotrace_sdk::EventMarkerColor::Hex(hex) = &color_style.color {
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
    // row is one sample; the metadata columns repeat and are read once.
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
            // No per-component trim: the other SDK ports split on ';' without
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

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    geotrace_sdk::Timestamp::try_from_iso8601(s)
        .ok()
        .map(Into::into)
}
