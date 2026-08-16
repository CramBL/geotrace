use chrono::{DateTime, Duration, Utc};
use egui::Ui;
use egui::{Grid, TextEdit};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROWS_OUT_SIMPLE as ICON_ARROWS_OUT_SIMPLE;
use egui_phosphor::regular::BOUNDING_BOX as ICON_BOUNDING_BOX;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use gt_filter::GlobalFilter;
use gt_types::{LoadedFile, MarkerRequirement};
use gt_ui_theme::EM_DASH;
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

/// Persistent state for raw text inputs in the filter panel.
/// Kept separate from `GlobalFilter` so it survives the parse round-trip.
#[derive(Debug, Default, Clone)]
pub struct FilterPanelState {
    pub distance_input: String,
    pub duration_input: String,
    pub spread_input: String,
    /// Stable viewport for the secondary (zoomed) time range bar.
    /// Initialised from the active data range the first time the secondary bar
    /// appears. Reset when the primary bar changes or filters are cleared.
    pub secondary_zoom: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

/// Render the global-filter controls. Returns `true` when the user clicked
/// "Reset filters", so the caller can also clear dependent state (the query
/// filter).
#[must_use]
pub fn render_filter_panel(
    ui: &mut Ui,
    files: &[LoadedFile],
    filter: &mut GlobalFilter,
    state: &mut FilterPanelState,
) -> bool {
    let full_range = compute_full_time_range(files);
    let filtered_range = compute_filtered_time_range(files, filter);

    if let Some((range_start, range_end)) = full_range {
        let sel_start = filter.time_start.unwrap_or(range_start);
        let sel_end = filter.time_end.unwrap_or(range_end);
        let dur_str = gt_fmt::format_human_terse_duration(sel_end - sel_start);
        ui.label(format!("Time range {EM_DASH} {dur_str}"));
        let primary_changed = time_range_bar(
            ui,
            (range_start, range_end),
            &mut filter.time_start,
            &mut filter.time_end,
        );
        if primary_changed {
            state.secondary_zoom = None;
        }

        // Secondary (zoomed) time range bar - shown when the active data range
        // is much narrower than the full range (e.g. one 30-minute track among
        // several days). Uses a stable stored viewport so that dragging its
        // handles doesn't shift the viewport under the cursor.
        if let Some((filt_start, filt_end)) = filtered_range {
            let full_secs = (range_end - range_start).num_seconds();
            let filt_secs = (filt_end - filt_start).num_seconds();
            if full_secs > 0 && filt_secs * 5 < full_secs {
                if state.secondary_zoom.is_none() {
                    state.secondary_zoom = Some(expand_range(
                        (filt_start, filt_end),
                        (range_start, range_end),
                    ));
                }
                if let Some((zoom_start, zoom_end)) = state.secondary_zoom {
                    let zoom_dur = gt_fmt::format_human_terse_duration(zoom_end - zoom_start);
                    ui.label(format!("Active range {EM_DASH} {zoom_dur}"));
                    time_range_bar(
                        ui,
                        (zoom_start, zoom_end),
                        &mut filter.time_start,
                        &mut filter.time_end,
                    );
                }
            } else {
                state.secondary_zoom = None;
            }
        } else {
            state.secondary_zoom = None;
        }
    }

    // Three-column grid: label | text edit | unit.
    let (dist_changed, dur_changed, spread_changed) = Grid::new("filter_inputs")
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            ui.label(format!("{ICON_ARROWS_OUT_SIMPLE} Min dist"));
            let dist = ui.add(TextEdit::singleline(&mut state.distance_input).desired_width(60.0));
            ui.label("km");
            ui.end_row();

            ui.label(format!("{ICON_CLOCK} Min dur"));
            let dur = ui.add(
                TextEdit::singleline(&mut state.duration_input)
                    .desired_width(60.0)
                    .hint_text("1h30m"),
            );
            ui.label(""); // no unit for duration
            ui.end_row();

            ui.label(format!("{ICON_BOUNDING_BOX} Min spread"));
            let spread = ui.add(TextEdit::singleline(&mut state.spread_input).desired_width(60.0));
            ui.label("m");
            ui.end_row();

            (dist.changed(), dur.changed(), spread.changed())
        })
        .inner;

    if dist_changed {
        filter.min_distance_km = state
            .distance_input
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|&v| v > 0.0)
            .map(Length::new::<kilometer>);
    }
    if dur_changed {
        filter.min_duration = parse_duration_input(&state.duration_input);
    }
    if spread_changed {
        filter.min_spread_m = state
            .spread_input
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|&v| v > 0.0)
            .map(Length::new::<meter>);
    }

    // Marker requirement - mutually exclusive options rendered as toggleable labels.
    let req = &mut filter.marker_requirement;
    ui.horizontal(|ui| {
        if ui
            .selectable_label(*req == MarkerRequirement::AnyMarker, "W/ markers only")
            .clicked()
        {
            *req = if *req == MarkerRequirement::AnyMarker {
                MarkerRequirement::None
            } else {
                MarkerRequirement::AnyMarker
            };
        }
        if ui
            .selectable_label(
                *req == MarkerRequirement::CustomMarker,
                "W/ custom markers only",
            )
            .clicked()
        {
            *req = if *req == MarkerRequirement::CustomMarker {
                MarkerRequirement::None
            } else {
                MarkerRequirement::CustomMarker
            };
        }
    });

    if ui
        .small_button(format!("{ICON_ARROW_COUNTER_CLOCKWISE} Reset filters"))
        .clicked()
    {
        *filter = GlobalFilter::default();
        *state = FilterPanelState::default();
        return true;
    }
    false
}

fn compute_full_time_range(files: &[LoadedFile]) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let mut min: Option<DateTime<Utc>> = None;
    let mut max: Option<DateTime<Utc>> = None;
    for file in files {
        let start = file.metadata.time_range.start;
        let end = file.metadata.time_range.end;
        min = Some(min.map_or(start, |m: DateTime<Utc>| m.min(start)));
        max = Some(max.map_or(end, |m: DateTime<Utc>| m.max(end)));
    }
    min.zip(max)
}

fn time_range_bar(
    ui: &mut Ui,
    full_range: (DateTime<Utc>, DateTime<Utc>),
    selected_start: &mut Option<DateTime<Utc>>,
    selected_end: &mut Option<DateTime<Utc>>,
) -> bool {
    let (full_start, full_end) = full_range;
    let total_secs = (full_end - full_start).num_seconds() as f64;
    if total_secs <= 0.0 {
        return false;
    }

    let desired_size = egui::vec2(ui.available_width(), 24.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

    let track_left = rect.left() + 8.0;
    let track_right = rect.right() - 8.0;
    let track_width = track_right - track_left;
    let track_y = rect.center().y;

    let painter = ui.painter();
    painter.rect_filled(
        egui::Rect::from_x_y_ranges(track_left..=track_right, track_y - 2.0..=track_y + 2.0),
        2.0,
        ui.visuals().widgets.inactive.bg_fill,
    );

    let to_x = |dt: DateTime<Utc>| -> f32 {
        let offset = (dt - full_start).num_seconds() as f64 / total_secs;
        track_left + (offset as f32) * track_width
    };
    let to_dt = |x: f32| -> DateTime<Utc> {
        let frac = ((x - track_left) / track_width).clamp(0.0, 1.0) as f64;
        let secs = (frac * total_secs) as i64;
        full_start + Duration::seconds(secs)
    };

    let start_x = selected_start.map_or(track_left, to_x);
    let end_x = selected_end.map_or(track_right, to_x);

    let fill_color = ui.visuals().selection.bg_fill;
    painter.rect_filled(
        egui::Rect::from_x_y_ranges(start_x..=end_x, track_y - 2.0..=track_y + 2.0),
        2.0,
        fill_color,
    );

    let handle_radius = 6.0;
    let handle_color = ui.visuals().widgets.active.fg_stroke.color;

    painter.circle_filled(egui::pos2(start_x, track_y), handle_radius, handle_color);
    painter.circle_filled(egui::pos2(end_x, track_y), handle_radius, handle_color);

    let mut changed = false;
    if let Some(pointer) = response.interact_pointer_pos() {
        let dist_start = (pointer.x - start_x).abs();
        let dist_end = (pointer.x - end_x).abs();
        if dist_start < dist_end {
            let new_dt = to_dt(pointer.x).min(selected_end.unwrap_or(full_end));
            *selected_start = if new_dt <= full_start {
                None
            } else {
                Some(new_dt)
            };
        } else {
            let new_dt = to_dt(pointer.x).max(selected_start.unwrap_or(full_start));
            *selected_end = if new_dt >= full_end {
                None
            } else {
                Some(new_dt)
            };
        }
        changed = true;
    }

    if response.double_clicked() {
        *selected_start = None;
        *selected_end = None;
        changed = true;
    }

    let start_label = selected_start.map_or_else(
        || full_start.format("%m/%d %H:%M").to_string(),
        |dt| dt.format("%m/%d %H:%M").to_string(),
    );
    let end_label = selected_end.map_or_else(
        || full_end.format("%m/%d %H:%M").to_string(),
        |dt| dt.format("%m/%d %H:%M").to_string(),
    );
    ui.label(format!("{start_label} {EM_DASH} {end_label}"));
    changed
}

fn compute_filtered_time_range(
    files: &[LoadedFile],
    filter: &GlobalFilter,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let mut min: Option<DateTime<Utc>> = None;
    let mut max: Option<DateTime<Utc>> = None;
    for file in files {
        for track in &file.tracks {
            if gt_filter::track_passes_filter(&track.metadata, filter) {
                let start = track.metadata.time_range.start;
                let end = track.metadata.time_range.end;
                min = Some(min.map_or(start, |m: DateTime<Utc>| m.min(start)));
                max = Some(max.map_or(end, |m: DateTime<Utc>| m.max(end)));
            }
        }
    }
    min.zip(max)
}

fn expand_range(
    range: (DateTime<Utc>, DateTime<Utc>),
    full: (DateTime<Utc>, DateTime<Utc>),
) -> (DateTime<Utc>, DateTime<Utc>) {
    let (start, end) = range;
    let (full_start, full_end) = full;
    let span_secs = (end - start).num_seconds().max(60);
    let padding = Duration::seconds(span_secs / 5);
    let expanded_start = (start - padding).max(full_start);
    let expanded_end = (end + padding).min(full_end);
    (expanded_start, expanded_end)
}

fn parse_duration_input(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total_secs: i64 = 0;
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else {
            let val: i64 = current.parse().ok()?;
            current.clear();
            total_secs += match ch {
                'h' | 'H' => val * 3600,
                'm' | 'M' => val * 60,
                's' | 'S' => val,
                _ => return None,
            };
        }
    }
    if !current.is_empty() {
        return None;
    }
    if total_secs > 0 {
        Some(Duration::seconds(total_secs))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::path::PathBuf;

    use gt_types::TimeRange;

    use super::*;

    #[test]
    fn parse_empty_is_none() {
        assert_eq!(parse_duration_input(""), None);
    }

    #[test]
    fn parse_hours_minutes_seconds() {
        assert_eq!(parse_duration_input("1h30m"), Some(Duration::seconds(5400)));
        assert_eq!(parse_duration_input("2h"), Some(Duration::seconds(7200)));
        assert_eq!(parse_duration_input("45s"), Some(Duration::seconds(45)));
        assert_eq!(
            parse_duration_input("1h30m15s"),
            Some(Duration::seconds(5415))
        );
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert_eq!(parse_duration_input("abc"), None);
        assert_eq!(parse_duration_input("1x"), None);
    }

    #[test]
    fn roundtrip_format_parse() {
        let fmt = |secs: i64| -> String {
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            let s = secs % 60;
            let mut out = String::new();
            if h > 0 {
                write!(out, "{h}h").ok();
            }
            if m > 0 {
                write!(out, "{m}m").ok();
            }
            if s > 0 || out.is_empty() {
                write!(out, "{s}s").ok();
            }
            out
        };
        for secs in [0i64, 45, 90, 3600, 5415, 7265] {
            let formatted = fmt(secs);
            let parsed = parse_duration_input(&formatted);
            if secs == 0 {
                assert_eq!(parsed, None);
            } else {
                assert_eq!(parsed, Some(Duration::seconds(secs)));
            }
        }
    }

    #[test]
    fn compute_range_empty_files() {
        assert!(compute_full_time_range(&[]).is_none());
    }

    #[test]
    fn compute_range_single_file() {
        use chrono::TimeZone;
        use gt_types::track::{FileMetadata, LoadedFile};
        let file = LoadedFile {
            metadata: FileMetadata {
                filename: "test.gtd".to_owned(),
                total_distance_km: Length::new::<kilometer>(1.0),
                total_duration: Duration::seconds(60),
                time_range: TimeRange::new(
                    Utc.timestamp_opt(0, 0).single().expect("valid"),
                    Utc.timestamp_opt(60, 0).single().expect("valid"),
                ),
                ..FileMetadata::default()
            },
            tracks: vec![],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from("test.gtd")),
            load_warnings: vec![],
        };
        let range = compute_full_time_range(&[file]);
        assert!(range.is_some());
        let (start, end) = range.expect("some");
        assert_eq!(start.timestamp(), 0);
        assert_eq!(end.timestamp(), 60);
    }
}
