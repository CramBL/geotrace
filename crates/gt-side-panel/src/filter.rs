use chrono::{DateTime, Duration, Utc};
use egui::Ui;
use egui::{Grid, TextEdit};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROWS_OUT_SIMPLE as ICON_ARROWS_OUT_SIMPLE;
use egui_phosphor::regular::BOUNDING_BOX as ICON_BOUNDING_BOX;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use gt_filter::GlobalFilter;
use gt_types::{LoadedFile, MarkerRequirement, TimeRange};
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
    pub secondary_zoom: Option<TimeRange>,
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

    if let Some(full_range) = full_range {
        let sel_start = filter.time_start.unwrap_or(full_range.start);
        let sel_end = filter.time_end.unwrap_or(full_range.end);
        let dur_str = gt_fmt::format_human_terse_duration(sel_end - sel_start);
        ui.label(format!("Time range {EM_DASH} {dur_str}"));
        let primary_changed =
            time_range_bar(ui, full_range, &mut filter.time_start, &mut filter.time_end);
        if primary_changed {
            state.secondary_zoom = None;
        }

        // Secondary (zoomed) time range bar - shown when the active data range
        // is much narrower than the full range (e.g. one 30-minute track among
        // several days). Uses a stable stored viewport so that dragging its
        // handles doesn't shift the viewport under the cursor.
        if let Some(filtered_range) = filtered_range {
            let full_seconds = full_range.duration().as_seconds_f64();
            let filtered_seconds = filtered_range.duration().as_seconds_f64();
            if full_seconds > 0.0 && filtered_seconds * 5.0 < full_seconds {
                let zoom_range = *state
                    .secondary_zoom
                    .get_or_insert_with(|| expand_range(filtered_range, full_range));
                let zoom_dur = gt_fmt::format_human_terse_duration(zoom_range.duration());
                ui.label(format!("Active range {EM_DASH} {zoom_dur}"));
                time_range_bar(ui, zoom_range, &mut filter.time_start, &mut filter.time_end);
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

fn compute_full_time_range(files: &[LoadedFile]) -> Option<TimeRange> {
    let mut min: Option<DateTime<Utc>> = None;
    let mut max: Option<DateTime<Utc>> = None;
    for file in files {
        let start = file.metadata.time_range.start;
        let end = file.metadata.time_range.end;
        min = Some(min.map_or(start, |m: DateTime<Utc>| m.min(start)));
        max = Some(max.map_or(end, |m: DateTime<Utc>| m.max(end)));
    }
    min.zip(max).map(|(start, end)| TimeRange::new(start, end))
}

/// Converts between an instant and its fraction of a time range bar's span,
/// in both directions.
#[derive(Debug, Clone, Copy)]
struct TimeRangeBarScale {
    range: TimeRange,
    span_seconds: f64,
}

impl TimeRangeBarScale {
    /// `None` for a range that ends where it begins or before it, which no bar
    /// can lay out.
    fn new(range: TimeRange) -> Option<Self> {
        let span_seconds = range.duration().as_seconds_f64();
        (span_seconds > 0.0).then_some(Self {
            range,
            span_seconds,
        })
    }

    fn fraction_at_instant(self, instant: DateTime<Utc>) -> f64 {
        (instant - self.range.start).as_seconds_f64() / self.span_seconds
    }

    /// The instant `fraction` of the way through the range, clamped to its
    /// ends. Resolved to the millisecond, the finest step an `i64` holds over
    /// the whole `DateTime<Utc>` range of ±262 000 years.
    fn instant_at_fraction(self, fraction: f64) -> DateTime<Utc> {
        let milliseconds = fraction.clamp(0.0, 1.0) * self.span_seconds * 1_000.0;
        self.range
            .start
            .checked_add_signed(Duration::milliseconds(milliseconds as i64))
            .unwrap_or(self.range.end)
    }
}

fn time_range_bar(
    ui: &mut Ui,
    full_range: TimeRange,
    selected_start: &mut Option<DateTime<Utc>>,
    selected_end: &mut Option<DateTime<Utc>>,
) -> bool {
    let Some(scale) = TimeRangeBarScale::new(full_range) else {
        return false;
    };

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
        track_left + (scale.fraction_at_instant(dt) as f32) * track_width
    };
    let to_dt = |x: f32| -> DateTime<Utc> {
        scale.instant_at_fraction(f64::from((x - track_left) / track_width))
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
            let new_dt = to_dt(pointer.x).min(selected_end.unwrap_or(full_range.end));
            *selected_start = if new_dt <= full_range.start {
                None
            } else {
                Some(new_dt)
            };
        } else {
            let new_dt = to_dt(pointer.x).max(selected_start.unwrap_or(full_range.start));
            *selected_end = if new_dt >= full_range.end {
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
        || full_range.start.format("%m/%d %H:%M").to_string(),
        |dt| dt.format("%m/%d %H:%M").to_string(),
    );
    let end_label = selected_end.map_or_else(
        || full_range.end.format("%m/%d %H:%M").to_string(),
        |dt| dt.format("%m/%d %H:%M").to_string(),
    );
    ui.label(format!("{start_label} {EM_DASH} {end_label}"));
    changed
}

fn compute_filtered_time_range(files: &[LoadedFile], filter: &GlobalFilter) -> Option<TimeRange> {
    let mut min: Option<DateTime<Utc>> = None;
    let mut max: Option<DateTime<Utc>> = None;
    for file in files {
        for track in &file.tracks {
            if gt_filter::track_passes_filter(track, filter) {
                let start = track.metadata.time_range.start;
                let end = track.metadata.time_range.end;
                min = Some(min.map_or(start, |m: DateTime<Utc>| m.min(start)));
                max = Some(max.map_or(end, |m: DateTime<Utc>| m.max(end)));
            }
        }
    }
    min.zip(max).map(|(start, end)| TimeRange::new(start, end))
}

fn expand_range(range: TimeRange, full: TimeRange) -> TimeRange {
    let span_secs = range.duration().num_seconds().max(60);
    let padding = Duration::seconds(span_secs / 5);
    TimeRange::new(
        (range.start - padding).max(full.start),
        (range.end + padding).min(full.end),
    )
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

    use rustc_hash::FxHashMap;

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
        use gt_types::track::{FileMetadata, LoadedFile, TotalDistance};
        let file = LoadedFile {
            metadata: FileMetadata {
                filename: "test.gtd".to_owned(),
                total_distance: TotalDistance::Measured(Length::new::<kilometer>(1.0)),
                total_duration: Duration::seconds(60),
                time_range: TimeRange::new(
                    Utc.timestamp_opt(0, 0).single().expect("valid"),
                    Utc.timestamp_opt(60, 0).single().expect("valid"),
                ),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: vec![],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from("test.gtd")),
            load_warnings: vec![],
        };
        let range = compute_full_time_range(&[file]).expect("one file has a time range");
        assert_eq!(range.start.timestamp(), 0);
        assert_eq!(range.end.timestamp(), 60);
    }

    /// Ten fixes at 10 Hz cover 900 ms, the span of a bar whose every handle
    /// position is a fraction of a second.
    fn sub_second_range() -> TimeRange {
        let start = DateTime::UNIX_EPOCH;
        TimeRange::new(start, start + Duration::milliseconds(900))
    }

    #[test]
    fn a_range_under_a_second_has_a_scale() {
        assert!(TimeRangeBarScale::new(sub_second_range()).is_some());
    }

    #[test]
    fn a_range_that_ends_where_it_begins_has_no_scale() {
        let start = DateTime::UNIX_EPOCH;
        assert!(TimeRangeBarScale::new(TimeRange::new(start, start)).is_none());
    }

    #[test]
    fn fraction_at_instant_keeps_the_sub_second_part() {
        let range = sub_second_range();
        let scale = TimeRangeBarScale::new(range).expect("a 900 ms range has a scale");
        let halfway = range.start + Duration::milliseconds(450);
        assert!((scale.fraction_at_instant(halfway) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn instant_at_fraction_keeps_the_sub_second_part() {
        let range = sub_second_range();
        let scale = TimeRangeBarScale::new(range).expect("a 900 ms range has a scale");
        assert_eq!(
            scale.instant_at_fraction(0.5),
            range.start + Duration::milliseconds(450)
        );
    }

    #[test]
    fn instant_at_fraction_clamps_to_the_ends_of_the_range() {
        let range = sub_second_range();
        let scale = TimeRangeBarScale::new(range).expect("a 900 ms range has a scale");
        assert_eq!(scale.instant_at_fraction(-0.5), range.start);
        assert_eq!(scale.instant_at_fraction(1.5), range.end);
    }

    /// A range of every instant `DateTime<Utc>` can hold, far longer than any
    /// recording, still lands within a second of its end.
    #[test]
    fn instant_at_fraction_stays_in_a_range_of_the_whole_datetime_span() {
        let range = TimeRange::new(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC);
        let scale = TimeRangeBarScale::new(range).expect("the widest range has a scale");
        let end = scale.instant_at_fraction(1.0);
        assert!(end <= range.end);
        assert!(range.end - end < Duration::seconds(1));
    }
}
