//! The backward time step overlay: a mark along the plot's bottom edge
//! wherever a channel's sample timestamps step back, plus its hover text.
//!
//! The samples stay in the order the file stored them. The plot draws one line
//! per chronological run, and a mark sits where two runs meet.

use chrono::TimeDelta;
use egui::epaint::{Shape, Stroke};
use egui::{Color32, Pos2};
use egui_plot::{PlotPoint, PlotTransform};

use super::chips::ChannelVisibility;
use super::lines::{ANOMALY_HOVER_RADIUS_PX, NearestHoverLabel, PlotHoverLabel, visible_by_x};
use super::overlay::{EDGE_INSET, OverlayItem, OverlayPainter, TAIL_LENGTH};
use crate::series::{ChannelSeries, PlacedBackwardTimeStep};

/// Screen distance between two marks, in points, below which the steps share
/// one mark. Wider than [`MARK_WIDTH`], which leaves a gap between two
/// neighbouring marks.
///
/// A jittering recorder clock steps back on a large fraction of its samples: a
/// channel sampled at 10 Hz for an hour holds 36 000 of them, far past the 100
/// marks a 1000 point wide plot fits at this pitch. Drawing each one paints a
/// solid band along the axis, which says less than the count in one mark's
/// hover text does.
const MARK_PITCH_PX: f32 = MARK_WIDTH + 4.0;

/// How the hover text writes the two timestamps of a step. Milliseconds: a
/// channel sampled faster than 1 Hz steps back by less than a second.
const STEP_TIME_FORMAT: &str = "%H:%M:%S%.3f";

/// How the hover text writes the two timestamps of a step under a millisecond,
/// which [`STEP_TIME_FORMAT`] renders as one and the same time. Microseconds
/// are the resolution the channel timestamps hold.
const SUB_MILLISECOND_STEP_TIME_FORMAT: &str = "%H:%M:%S%.6f";

/// One backward time step together with the channel it belongs to.
struct ChannelBackwardTimeStep<'a> {
    channel: &'a str,
    placed: PlacedBackwardTimeStep,
}

/// Length of the leader descending to the step, in points. The same as the
/// clock excursion marker's tail, so the two overlays' marks reach equally far
/// into the plot.
const LEADER_LENGTH: f32 = TAIL_LENGTH;

/// Half the width of the step's horizontal run, in points. The leader descends
/// this far right of the anchor, and the drop falls this far left of it.
const STEP_HALF_WIDTH: f32 = 2.5;

/// How far above the anchor the step's horizontal run sits, in points.
const STEP_ABOVE_ANCHOR: f32 = 1.0;

/// How far below the anchor the drop ends, in points.
const DROP_BELOW_ANCHOR: f32 = 2.5;

/// Stroke width of the mark, in points.
const MARK_STROKE_WIDTH: f32 = 1.0;

/// Width a mark covers, in points.
const MARK_WIDTH: f32 = 2.0 * STEP_HALF_WIDTH + MARK_STROKE_WIDTH;

/// How far above the bottom of the view a mark's anchor sits at the least, in
/// points. [`EDGE_INSET`] is a fraction of the visible y range: that fraction
/// of a short view is less than the drop below the anchor.
const MIN_MARK_HEIGHT_ABOVE_EDGE: f32 = DROP_BELOW_ANCHOR + MARK_STROKE_WIDTH;

/// The marks of one track's channels.
struct BackwardTimeStepMarks {
    /// Where each mark is anchored, in ascending x.
    anchors: Vec<PlotPoint>,
    color: Color32,
}

impl OverlayPainter for BackwardTimeStepMarks {
    fn legend_color(&self) -> Color32 {
        self.color
    }

    /// One stroked path per mark: the leader descends right of the anchor,
    /// turns left, and drops past it.
    fn paint(&self, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let stroke = Stroke::new(MARK_STROKE_WIDTH, self.color);
        for anchor in &self.anchors {
            let at = transform.position_from_point(anchor);
            shapes.push(Shape::line(
                vec![
                    Pos2::new(at.x + STEP_HALF_WIDTH, at.y - LEADER_LENGTH),
                    Pos2::new(at.x + STEP_HALF_WIDTH, at.y - STEP_ABOVE_ANCHOR),
                    Pos2::new(at.x - STEP_HALF_WIDTH, at.y - STEP_ABOVE_ANCHOR),
                    Pos2::new(at.x - STEP_HALF_WIDTH, at.y + DROP_BELOW_ANCHOR),
                ],
                stroke,
            ));
        }
    }
}

/// The frame-level inputs the overlay needs beyond the channels themselves:
/// the visible x range it clips to, the two gates it draws under, and the
/// theme.
#[derive(Clone, Copy)]
pub(super) struct BackwardTimeStepViewport<'v> {
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    /// Whether the marks are drawn at all: the Channels section reveals them
    /// and the Settings toggle turns them off.
    pub(super) marks_shown: bool,
    pub(super) channel_vis: &'v ChannelVisibility,
    pub(super) dark_mode: bool,
}

/// Draw the backward time step marks of one track's channels and, when the
/// pointer is within [`ANOMALY_HOVER_RADIUS_PX`] of one, record the nearest in
/// `nearest` so the caller can show its tooltip.
///
/// The marks follow the same gates as the channel lines they annotate: with
/// the Channels section collapsed this draws nothing, and a channel the user
/// hid contributes none.
pub(super) fn add_backward_time_steps(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    channels: &[ChannelSeries],
    track_label: Option<&str>,
    viewport: BackwardTimeStepViewport<'_>,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
) {
    if !viewport.marks_shown {
        return;
    }
    let mut steps: Vec<ChannelBackwardTimeStep<'_>> = Vec::new();
    for channel in channels {
        if !viewport.channel_vis.is_visible(&channel.name) {
            continue;
        }
        for placed in visible_by_x(
            &channel.backward_time_steps,
            |placed| placed.x_secs,
            viewport.x_min,
            viewport.x_max,
        ) {
            steps.push(ChannelBackwardTimeStep {
                channel: &channel.name,
                placed: *placed,
            });
        }
    }
    if steps.is_empty() {
        return;
    }
    steps.sort_by(|a, b| a.placed.x_secs.total_cmp(&b.placed.x_secs));

    let bounds = plot_ui.plot_bounds();
    let [_, y_min] = bounds.min();
    let [_, y_max] = bounds.max();
    let view = VerticalView {
        height: y_max - y_min,
        plot_units_per_point: plot_ui.transform().dvalue_dpos()[1].abs(),
    };
    let y = y_min + view.mark_height_above_edge();
    let marks = group_into_marks(&steps, |x_secs| {
        plot_ui.screen_from_plot(PlotPoint::new(x_secs, y)).x
    });

    plot_ui.add(OverlayItem::new(
        "Backward time step",
        BackwardTimeStepMarks {
            anchors: marks
                .iter()
                .filter_map(|mark| mark.first())
                .map(|first| PlotPoint::new(first.placed.x_secs, y))
                .collect(),
            color: gt_ui_theme::warning_amber(viewport.dark_mode),
        },
    ));

    let Some(pointer) = pointer else {
        return;
    };
    for mark in marks {
        let Some(first) = mark.first() else {
            continue;
        };
        let anchor = plot_ui.screen_from_plot(PlotPoint::new(first.placed.x_secs, y));
        let distance = MarkHover { pointer, anchor }.distance_px();
        if distance <= ANOMALY_HOVER_RADIUS_PX {
            nearest.offer(distance, || {
                PlotHoverLabel::BackwardTimeStep(BackwardTimeStepHover::new(track_label, mark))
            });
        }
    }
}

/// The plot's visible y range, in the two terms placing a mark takes.
#[derive(Clone, Copy)]
struct VerticalView {
    /// Height of the range, in plot units.
    height: f64,
    /// Plot units one screen point covers on the y axis.
    plot_units_per_point: f64,
}

impl VerticalView {
    /// How far above the bottom of the view a mark's anchor sits, in plot
    /// units: [`EDGE_INSET`] of the view's height, and never less than
    /// [`MIN_MARK_HEIGHT_ABOVE_EDGE`] points.
    fn mark_height_above_edge(self) -> f64 {
        (self.height * EDGE_INSET)
            .max(self.plot_units_per_point * f64::from(MIN_MARK_HEIGHT_ABOVE_EDGE))
    }
}

/// The marks drawn for `steps`, which are in ascending x: a step opens a new
/// mark when it lands more than [`MARK_PITCH_PX`] from the step that opened
/// the mark before it, and joins that mark otherwise.
fn group_into_marks<'s, 'a>(
    steps: &'s [ChannelBackwardTimeStep<'a>],
    screen_x: impl Fn(f64) -> f32,
) -> Vec<&'s [ChannelBackwardTimeStep<'a>]> {
    let mut marks = Vec::new();
    let mut start = 0;
    let mut anchor: Option<f32> = None;
    for (index, step) in steps.iter().enumerate() {
        let x = screen_x(step.placed.x_secs);
        if anchor.is_some_and(|anchor_x| x - anchor_x <= MARK_PITCH_PX) {
            continue;
        }
        if let Some(mark) = steps.get(start..index)
            && !mark.is_empty()
        {
            marks.push(mark);
        }
        start = index;
        anchor = Some(x);
    }
    if let Some(mark) = steps.get(start..)
        && !mark.is_empty()
    {
        marks.push(mark);
    }
    marks
}

/// The pointer and one mark's anchor, both in screen points.
#[derive(Clone, Copy)]
struct MarkHover {
    pointer: Pos2,
    anchor: Pos2,
}

impl MarkHover {
    /// Distance from the pointer to the box the mark's path spans, which is
    /// zero anywhere on the leader, the step and the drop.
    fn distance_px(self) -> f32 {
        let dx = self.pointer.x
            - self.pointer.x.clamp(
                self.anchor.x - STEP_HALF_WIDTH,
                self.anchor.x + STEP_HALF_WIDTH,
            );
        let dy = self.pointer.y
            - self.pointer.y.clamp(
                self.anchor.y - LEADER_LENGTH,
                self.anchor.y + DROP_BELOW_ANCHOR,
            );
        dx.hypot(dy)
    }
}

/// Pre-formatted tooltip contents for one backward time step mark.
pub(super) struct BackwardTimeStepHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    /// How many steps the mark stands for, over all its channels.
    step_count: usize,
    /// One line per channel of the mark, sorted by channel name.
    channels: Vec<String>,
}

impl BackwardTimeStepHover {
    fn new(track_label: Option<&str>, mark: &[ChannelBackwardTimeStep<'_>]) -> Self {
        let mut names: Vec<&str> = mark.iter().map(|step| step.channel).collect();
        names.sort_unstable();
        names.dedup();
        let channels = names
            .into_iter()
            .map(|name| {
                let steps: Vec<PlacedBackwardTimeStep> = mark
                    .iter()
                    .filter(|step| step.channel == name)
                    .map(|step| step.placed)
                    .collect();
                channel_line(name, &steps)
            })
            .collect();
        Self {
            track: track_label.map(ToOwned::to_owned),
            step_count: mark.len(),
            channels,
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        if self.step_count == 1 {
            ui.strong("Backward time step");
        } else {
            ui.strong("Backward time steps");
        }
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.separator();
        for line in &self.channels {
            ui.label(line);
        }
        ui.separator();
        ui.label("A time sync on the recorder, such as NTP, is the usual cause.");
        ui.label("The samples stay in the order the file stored them.");
    }
}

/// What the hover says about one channel of a mark: the two timestamps of a
/// single step, or the count and the largest step of several.
fn channel_line(channel: &str, steps: &[PlacedBackwardTimeStep]) -> String {
    let largest_step_back = gt_fmt::format_human_terse_duration_with_microseconds(
        steps
            .iter()
            .map(|placed| placed.step_back())
            .max()
            .unwrap_or_default(),
    );
    match steps {
        [only] => {
            let time_format = if only.step_back() < TimeDelta::milliseconds(1) {
                SUB_MILLISECOND_STEP_TIME_FORMAT
            } else {
                STEP_TIME_FORMAT
            };
            format!(
                "{channel} - {} {} {}, {largest_step_back} back",
                only.previous_time.format(time_format),
                gt_fmt::RIGHTWARDS_ARROW,
                only.time.format(time_format),
            )
        }
        _ => format!(
            "{channel} - {} steps, largest {largest_step_back} back",
            steps.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;

    /// 2024-01-15 12:00:00 UTC, where the channels below are sampled.
    const T: f64 = 1_705_320_000.0;

    fn at(offset: TimeDelta) -> DateTime<Utc> {
        DateTime::from_timestamp(T as i64, 0).unwrap_or_default() + offset
    }

    /// Where a step sits, and how far its sample clock stepped back.
    #[derive(Clone, Copy)]
    struct StepSpec {
        offset: TimeDelta,
        back: TimeDelta,
    }

    fn step(spec: StepSpec) -> PlacedBackwardTimeStep {
        PlacedBackwardTimeStep {
            previous_time: at(spec.offset + spec.back),
            time: at(spec.offset),
            x_secs: T + spec.offset.as_seconds_f64(),
        }
    }

    fn channel_step<'a>(
        channel: &'a str,
        placed: PlacedBackwardTimeStep,
    ) -> ChannelBackwardTimeStep<'a> {
        ChannelBackwardTimeStep { channel, placed }
    }

    /// In the grouping tests a step's x is its screen distance: one point of
    /// plot x per point of screen.
    fn one_pixel_per_second(x_secs: f64) -> f32 {
        (x_secs - T) as f32
    }

    /// A tall view holds the whole mark at [`EDGE_INSET`] of its height, and a
    /// short one raises the anchor to [`MIN_MARK_HEIGHT_ABOVE_EDGE`] points.
    #[rstest::rstest]
    #[case::tall_view(400.0, 12.0)]
    #[case::short_view(80.0, 3.5)]
    fn a_short_view_raises_the_mark_off_its_bottom_edge(
        #[case] view_height_points: f64,
        #[case] expected_points_above_the_edge: f64,
    ) {
        // The height reads in points: one plot unit per screen point.
        let height = VerticalView {
            height: view_height_points,
            plot_units_per_point: 1.0,
        }
        .mark_height_above_edge();

        assert!(
            (height - expected_points_above_the_edge).abs() < 1e-6,
            "mark sits {height} points above the edge, expected {expected_points_above_the_edge}"
        );
    }

    /// The steps a jittering clock leaves land within a pixel or two of each
    /// other, and one mark per glyph pitch is what the axis has room for.
    #[test]
    fn steps_inside_one_pitch_share_a_mark() {
        let dense: Vec<ChannelBackwardTimeStep<'_>> = (0..40)
            .map(|i| {
                channel_step(
                    "accel",
                    step(StepSpec {
                        offset: TimeDelta::milliseconds(i64::from(i) * 200),
                        back: TimeDelta::seconds(1),
                    }),
                )
            })
            .collect();

        let marks = group_into_marks(&dense, one_pixel_per_second);

        assert_eq!(marks.len(), 1);
        assert_eq!(marks.first().map(|mark| mark.len()), Some(40));
    }

    #[test]
    fn a_step_past_the_pitch_opens_the_next_mark() {
        let one_second_back = |offset_secs: i64| StepSpec {
            offset: TimeDelta::seconds(offset_secs),
            back: TimeDelta::seconds(1),
        };
        let pitch = MARK_PITCH_PX as i64;
        let steps = [
            channel_step("accel", step(one_second_back(0))),
            channel_step("accel", step(one_second_back(pitch - 1))),
            channel_step("accel", step(one_second_back(pitch + 1))),
            channel_step("accel", step(one_second_back(pitch * 4))),
        ];

        let marks = group_into_marks(&steps, one_pixel_per_second);

        let widths: Vec<usize> = marks.iter().map(|mark| mark.len()).collect();
        assert_eq!(widths, vec![2, 1, 1]);
    }

    /// The pitch is the largest distance that still joins: a step exactly that
    /// far from the step which opened the mark shares it.
    #[test]
    fn a_step_a_whole_pitch_from_the_anchor_shares_its_mark() {
        let steps = [
            channel_step(
                "accel",
                step(StepSpec {
                    offset: TimeDelta::zero(),
                    back: TimeDelta::seconds(1),
                }),
            ),
            channel_step(
                "accel",
                step(StepSpec {
                    offset: TimeDelta::seconds(MARK_PITCH_PX as i64),
                    back: TimeDelta::seconds(1),
                }),
            ),
        ];

        let marks = group_into_marks(&steps, one_pixel_per_second);

        assert_eq!(marks.len(), 1);
    }

    #[test]
    fn no_step_is_no_mark() {
        assert!(group_into_marks(&[], one_pixel_per_second).is_empty());
    }

    #[test]
    fn the_hover_reports_one_step_as_both_its_timestamps() {
        let steps = [channel_step(
            "accel",
            step(StepSpec {
                offset: TimeDelta::zero(),
                back: TimeDelta::seconds(4),
            }),
        )];

        let hover = BackwardTimeStepHover::new(Some("ride.gtd"), &steps);

        assert_eq!(hover.track.as_deref(), Some("ride.gtd"));
        assert_eq!(hover.step_count, 1);
        assert_eq!(
            hover.channels,
            ["accel - 12:00:04.000 → 12:00:00.000, 4s back"]
        );
    }

    /// A mark covering several channels reports each of them, with the count
    /// and the largest step of the channels that stepped back more than once.
    #[test]
    fn the_hover_reports_every_channel_of_a_mark() {
        let steps = [
            channel_step(
                "gyro",
                step(StepSpec {
                    offset: TimeDelta::milliseconds(100),
                    back: TimeDelta::milliseconds(250),
                }),
            ),
            channel_step(
                "accel",
                step(StepSpec {
                    offset: TimeDelta::zero(),
                    back: TimeDelta::seconds(4),
                }),
            ),
            channel_step(
                "accel",
                step(StepSpec {
                    offset: TimeDelta::milliseconds(200),
                    back: TimeDelta::seconds(9),
                }),
            ),
        ];

        let hover = BackwardTimeStepHover::new(None, &steps);

        assert_eq!(hover.step_count, 3);
        assert_eq!(
            hover.channels,
            [
                "accel - 2 steps, largest 9s back",
                "gyro - 12:00:00.350 → 12:00:00.100, 250ms back",
            ]
        );
    }

    /// A step under a second reads at its own scale, and one under a
    /// millisecond widens both timestamps to microseconds, which
    /// [`STEP_TIME_FORMAT`] would print as one and the same time.
    #[rstest::rstest]
    #[case::milliseconds(
        TimeDelta::milliseconds(4),
        "accel - 12:00:04.004 → 12:00:04.000, 4ms back"
    )]
    #[case::microseconds(
        TimeDelta::microseconds(900),
        "accel - 12:00:04.000900 → 12:00:04.000000, 900µs back"
    )]
    fn the_hover_reports_a_sub_second_step_at_its_own_scale(
        #[case] back: TimeDelta,
        #[case] expected: &str,
    ) {
        let steps = [channel_step(
            "accel",
            step(StepSpec {
                offset: TimeDelta::seconds(4),
                back,
            }),
        )];

        let hover = BackwardTimeStepHover::new(None, &steps);

        assert_eq!(hover.channels, [expected]);
    }

    /// The pointer catches a mark anywhere along its path, and the path runs
    /// from 22 points above the anchor to 2.5 below it, 2.5 either side.
    #[rstest::rstest]
    #[case::on_the_leader(egui::pos2(102.5, 190.0), 0.0)]
    #[case::above_the_leader(egui::pos2(102.5, 168.0), 10.0)]
    #[case::below_the_drop(egui::pos2(97.5, 206.5), 4.0)]
    #[case::beside_the_mark(egui::pos2(108.5, 190.0), 6.0)]
    #[case::past_the_corner(egui::pos2(105.5, 174.0), 5.0)]
    fn the_pointer_catches_the_mark_anywhere_along_its_path(
        #[case] pointer: Pos2,
        #[case] expected: f32,
    ) {
        let distance = MarkHover {
            pointer,
            anchor: egui::pos2(100.0, 200.0),
        }
        .distance_px();

        assert!(
            (distance - expected).abs() < 1e-6,
            "distance is {distance}, expected {expected}"
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use chrono::DateTime;
    use egui_plot::{Line, PlotBounds, PlotPoints};
    use gt_test_utils::TestHarness;
    use rstest::rstest;

    use super::*;

    /// 2024-01-15 12:00:00 UTC, the plot's left edge.
    const T: f64 = 1_705_320_000.0;

    /// The plot's right edge, a minute later.
    const T_END: f64 = T + 60.0;

    /// A step at `offset_secs` seconds from the left edge. Its sample clock
    /// stepped one second back.
    fn step(offset_secs: f64) -> PlacedBackwardTimeStep {
        let time = DateTime::from_timestamp(T as i64 + offset_secs as i64, 0).unwrap_or_default();
        PlacedBackwardTimeStep {
            previous_time: time + chrono::TimeDelta::seconds(1),
            time,
            x_secs: T + offset_secs,
        }
    }

    fn channel(name: &str, offsets_secs: &[f64]) -> ChannelSeries {
        ChannelSeries {
            name: name.to_owned(),
            unit: None,
            components: Vec::new(),
            backward_time_steps: offsets_secs.iter().map(|&offset| step(offset)).collect(),
        }
    }

    /// One isolated step, a second one a channel away, and a jittering stretch
    /// whose steps land inside a pitch of each other.
    fn channels() -> Vec<ChannelSeries> {
        let jitter: Vec<f64> = (0..40).map(|i| 40.0 + f64::from(i) * 0.2).collect();
        vec![
            channel("accel", &[10.0]),
            channel("gyro", &[[25.0].as_slice(), &jitter].concat()),
        ]
    }

    #[rstest]
    #[case::dark("backward_time_step_marks_dark", true)]
    #[case::light("backward_time_step_marks_light", false)]
    fn backward_time_step_marks(#[case] name: &str, #[case] dark_mode: bool) {
        let channels = channels();
        let channel_vis = ChannelVisibility::default();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(420.0, 220.0))
            .theme(dark_mode)
            .ui(|ui| {
                egui_plot::Plot::new("backward_time_step_marks")
                    .show_grid(false)
                    .show(ui, |plot_ui| {
                        plot_ui.set_plot_bounds(PlotBounds::from_min_max([T, 0.0], [T_END, 10.0]));
                        plot_ui.line(Line::new(
                            "Channel",
                            PlotPoints::new(vec![[T, 2.0], [T_END, 6.0]]),
                        ));
                        add_backward_time_steps(
                            plot_ui,
                            &channels,
                            None,
                            BackwardTimeStepViewport {
                                x_min: T,
                                x_max: T_END,
                                marks_shown: true,
                                channel_vis: &channel_vis,
                                dark_mode,
                            },
                            None,
                            &mut NearestHoverLabel::default(),
                        );
                    });
            });
        harness.run();
        harness.snapshot_loose(name);
    }
}
