use chrono::{DateTime, Duration, Utc};
use egui::Ui;
use nav_types::{GlobalFilter, LoadedFile};

const EM_DASH: &str = "—";

/// Persistent state for raw text inputs in the filter panel.
/// Kept separate from `GlobalFilter` so it survives the parse round-trip.
#[derive(Debug, Default, Clone)]
pub struct FilterPanelState {
    pub distance_input: String,
    pub duration_input: String,
    pub spread_input: String,
}

pub fn render_filter_panel(
    ui: &mut Ui,
    files: &[LoadedFile],
    filter: &mut GlobalFilter,
    state: &mut FilterPanelState,
) {
    let full_range = compute_full_time_range(files);

    if let Some((range_start, range_end)) = full_range {
        let sel_start = filter.time_start.unwrap_or(range_start);
        let sel_end = filter.time_end.unwrap_or(range_end);
        let dur_str = nav_fmt::format_human_terse_duration(sel_end - sel_start);
        ui.label(format!("Time range {EM_DASH} {dur_str}"));
        time_range_bar(
            ui,
            (range_start, range_end),
            &mut filter.time_start,
            &mut filter.time_end,
        );
    }

    // Three-column grid: label | text edit | unit.  All filter inputs in one
    // aligned block so they line up neatly regardless of label length.
    let (dist_changed, dur_changed, spread_changed) = egui::Grid::new("filter_inputs")
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            ui.label("Min dist");
            let dist =
                ui.add(egui::TextEdit::singleline(&mut state.distance_input).desired_width(60.0));
            ui.label("km");
            ui.end_row();

            ui.label("Min dur");
            let dur = ui.add(
                egui::TextEdit::singleline(&mut state.duration_input)
                    .desired_width(60.0)
                    .hint_text("1h30m"),
            );
            ui.label(""); // no unit for duration
            ui.end_row();

            ui.label("Min spread");
            let spread =
                ui.add(egui::TextEdit::singleline(&mut state.spread_input).desired_width(60.0));
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
            .filter(|&v| v > 0.0);
    }
    if dur_changed {
        filter.min_duration_secs = parse_duration_input(&state.duration_input);
    }
    if spread_changed {
        filter.min_spread_m = state
            .spread_input
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|&v| v > 0.0);
    }

    // Marker filters — always visible so users know they exist before loading data.
    ui.checkbox(&mut filter.require_any_marker, "W/ markers only");
    ui.checkbox(&mut filter.require_custom_marker, "W/ custom markers only");

    if ui.small_button("Reset filters").clicked() {
        *filter = GlobalFilter::default();
        *state = FilterPanelState::default();
    }
}

fn compute_full_time_range(files: &[LoadedFile]) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let mut min: Option<DateTime<Utc>> = None;
    let mut max: Option<DateTime<Utc>> = None;
    for file in files {
        let start = file.metadata.time_range.0;
        let end = file.metadata.time_range.1;
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
) {
    let (full_start, full_end) = full_range;
    let total_secs = (full_end - full_start).num_seconds() as f64;
    if total_secs <= 0.0 {
        return;
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
    }

    if response.double_clicked() {
        *selected_start = None;
        *selected_end = None;
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
}

fn parse_duration_input(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total: i64 = 0;
    let mut current = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else {
            let val: i64 = current.parse().ok()?;
            current.clear();
            total += match ch {
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
    if total > 0 { Some(total) } else { None }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn parse_empty_is_none() {
        assert_eq!(parse_duration_input(""), None);
    }

    #[test]
    fn parse_hours_minutes_seconds() {
        assert_eq!(parse_duration_input("1h30m"), Some(5400));
        assert_eq!(parse_duration_input("2h"), Some(7200));
        assert_eq!(parse_duration_input("45s"), Some(45));
        assert_eq!(parse_duration_input("1h30m15s"), Some(5415));
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
                write!(out, "{h}h").unwrap_or(());
            }
            if m > 0 {
                write!(out, "{m}m").unwrap_or(());
            }
            if s > 0 || out.is_empty() {
                write!(out, "{s}s").unwrap_or(());
            }
            out
        };
        for secs in [0i64, 45, 90, 3600, 5415, 7265] {
            let formatted = fmt(secs);
            let parsed = parse_duration_input(&formatted);
            if secs == 0 {
                assert_eq!(parsed, None);
            } else {
                assert_eq!(parsed, Some(secs));
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
        use nav_types::trip::{FileMetadata, LoadedFile};
        let file = LoadedFile {
            metadata: FileMetadata {
                filename: "test.nvd".to_owned(),
                total_distance_km: 1.0,
                total_duration: Duration::seconds(60),
                time_range: (
                    Utc.timestamp_opt(0, 0).single().expect("valid"),
                    Utc.timestamp_opt(60, 0).single().expect("valid"),
                ),
            },
            trips: vec![],
        };
        let range = compute_full_time_range(&[file]);
        assert!(range.is_some());
        let (start, end) = range.expect("some");
        assert_eq!(start.timestamp(), 0);
        assert_eq!(end.timestamp(), 60);
    }
}
