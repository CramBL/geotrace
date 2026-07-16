use crate::AnalysisConfig;
use crate::series::{TrackSeries, build_all_series};
use chrono::{DateTime, Utc};
use egui::Color32;
use egui::{Area, Button, Frame, Label, RichText, Slider, Tooltip};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::GAUGE as ICON_GAUGE;
use egui_phosphor::regular::GEAR as ICON_GEAR;
use egui_phosphor::regular::LINK as ICON_LINK;
use egui_phosphor::regular::WAVE_SINE as ICON_WAVE_SINE;
use egui_plot::{Line, LineStyle, MarkerShape, PlotPoint, PlotPoints, Points, Span, VLine};
use gt_analysis::satellite_utilization::UtilAnomaly;
use gt_egui_mipmap::{LevelSelection, MipMap};
use gt_filter::GlobalFilter;
use gt_types::satellites::Constellation;
use gt_types::{FileIdx, LoadedFile, MetricKind, PointIdx, TrackIdx, TrackRef};
use gt_ui_types::{
    ArcIdentity, HighlightScope, SnapErrorKind, SnapErrorPoint, SnapErrorSeries,
    TrackDataVisibility,
};
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::num::NonZeroUsize;
use strum::IntoEnumIterator;

/// Pixel radius within which the pointer is considered to be hovering a
/// masked-satellite anomaly marker.
const ANOMALY_HOVER_RADIUS_PX: f32 = 7.0;
/// On-plot radius of the anomaly cross marker.
const ANOMALY_MARKER_RADIUS: f32 = 4.0;
/// Gap between the pointer and the anomaly hover tooltip.
const ANOMALY_TOOLTIP_GAP: f32 = 12.0;

/// Chip color, label, and optional hover tooltip for each [`MetricKind`].
///
/// `MetricKind` lives in `gt_types` (shared with the persisted settings, see
/// `geotrace::settings::PlotSettings::metric`). These are presentation
/// details specific to this widget, so they live here as an extension trait
/// rather than on the type itself. Together with the `match` in
/// [`MetricVisibility::field`] and [`MetricVisibility::field_mut`], adding a
/// variant forces a compile error here until every arm is filled in.
trait MetricKindUi {
    fn label(self) -> &'static str;
    fn hover_text(self) -> Option<&'static str>;
    /// Whether this metric belongs to the advanced analysis group, hidden behind
    /// the "Advanced" toggle in the chip row and off by default.
    fn is_advanced(&self) -> bool;
    /// The constellation this metric is specific to, or `None` for metrics that
    /// span all constellations (totals, velocity, EPH, …).  Used to gate
    /// per-constellation chips and lines on whether that constellation appears
    /// in the loaded data.
    fn constellation(self) -> Option<Constellation>;
}

impl MetricKindUi for MetricKind {
    fn constellation(self) -> Option<Constellation> {
        match self {
            Self::GpsSeen | Self::GpsFix | Self::UtilGps | Self::SlipGps => {
                Some(Constellation::Gps)
            }
            Self::GlonassSeen | Self::GlonassFix | Self::UtilGlonass | Self::SlipGlonass => {
                Some(Constellation::Glonass)
            }
            Self::GalileoSeen | Self::GalileoFix | Self::UtilGalileo | Self::SlipGalileo => {
                Some(Constellation::Galileo)
            }
            Self::BeidouSeen | Self::BeidouFix | Self::UtilBeidou | Self::SlipBeidou => {
                Some(Constellation::Beidou)
            }
            Self::NavicSeen | Self::NavicFix | Self::UtilNavic | Self::SlipNavic => {
                Some(Constellation::Navic)
            }
            Self::QzssSeen | Self::QzssFix | Self::UtilQzss | Self::SlipQzss => {
                Some(Constellation::Qzss)
            }
            Self::SatsSeen
            | Self::SatsFix
            | Self::Velocity
            | Self::Eph
            | Self::HeadingDeg
            | Self::ClockDeltaMs
            | Self::UtilAll
            | Self::SlipAll
            | Self::SnapError => None,
        }
    }

    fn is_advanced(&self) -> bool {
        matches!(
            self,
            Self::UtilAll
                | Self::UtilGps
                | Self::UtilGlonass
                | Self::UtilGalileo
                | Self::UtilBeidou
                | Self::UtilNavic
                | Self::UtilQzss
                | Self::SlipAll
                | Self::SlipGps
                | Self::SlipGlonass
                | Self::SlipGalileo
                | Self::SlipBeidou
                | Self::SlipNavic
                | Self::SlipQzss
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::SatsSeen => "Sats seen",
            Self::SatsFix => "Sats fix",
            Self::GpsSeen => "GPS seen",
            Self::GpsFix => "GPS fix",
            Self::GlonassSeen => "GLONASS seen",
            Self::GlonassFix => "GLONASS fix",
            Self::GalileoSeen => "Galileo seen",
            Self::GalileoFix => "Galileo fix",
            Self::BeidouSeen => "BeiDou seen",
            Self::BeidouFix => "BeiDou fix",
            Self::NavicSeen => "NavIC seen",
            Self::NavicFix => "NavIC fix",
            Self::QzssSeen => "QZSS seen",
            Self::QzssFix => "QZSS fix",
            Self::Velocity => "Velocity (km/h)",
            Self::Eph => "EPH (m)",
            Self::HeadingDeg => "Heading (°)",
            Self::ClockDeltaMs => "Clock Δt (ms)",
            Self::UtilAll => "Util all (%)",
            Self::UtilGps => "GPS util (%)",
            Self::UtilGlonass => "GLONASS util (%)",
            Self::UtilGalileo => "Galileo util (%)",
            Self::UtilBeidou => "BeiDou util (%)",
            Self::UtilNavic => "NavIC util (%)",
            Self::UtilQzss => "QZSS util (%)",
            Self::SlipAll => "Slip all (/min)",
            Self::SlipGps => "GPS slip (/min)",
            Self::SlipGlonass => "GLONASS slip (/min)",
            Self::SlipGalileo => "Galileo slip (/min)",
            Self::SlipBeidou => "BeiDou slip (/min)",
            Self::SlipNavic => "NavIC slip (/min)",
            Self::SlipQzss => "QZSS slip (/min)",
            Self::SnapError => "Snap error (m)",
        }
    }

    fn hover_text(self) -> Option<&'static str> {
        match self {
            Self::Eph => Some(
                "Estimated Horizontal Position error - the GPS receiver's own estimate of how \
                 far the reported position may be from the true position, in metres. \
                 Lower is more accurate.",
            ),
            Self::SnapError => Some(
                "Distance from each recorded point to its road-snapped position, in metres - \
                 the observed deviation from the road network. Plot it next to EPH to compare \
                 the receiver's claimed accuracy with the observed deviation. Values exist only \
                 for points sent in a completed snap run. Zoomed in, a dot marks a point the \
                 matcher placed independently; the plain line between dots is interpolated \
                 along the road; a cross at the baseline is a point the road network rejected.",
            ),
            Self::ClockDeltaMs => Some(
                "GPS clock lead over the host system clock, in milliseconds. \
                 Positive = GPS clock ahead of the system clock; negative = system clock ahead. \
                 Only shown when the receiver reports a system timestamp alongside the GPS fix.",
            ),
            Self::UtilAll => Some(
                "Utilization rate, all constellations: satellites used in the fix divided by \
                 satellites in view, both counted above the elevation mask. A red cross marks \
                 where a used satellite fell below the mask and was excluded. Adjust the mask in \
                 Settings.",
            ),
            Self::UtilGps => Some(
                "GPS utilization rate: GPS satellites used in the fix divided by GPS satellites \
                 in view above the elevation mask.",
            ),
            Self::UtilGlonass => Some(
                "GLONASS utilization rate: GLONASS satellites used in the fix divided by GLONASS \
                 satellites in view above the elevation mask.",
            ),
            Self::UtilGalileo => Some(
                "Galileo utilization rate: Galileo satellites used in the fix divided by Galileo \
                 satellites in view above the elevation mask.",
            ),
            Self::UtilBeidou => Some(
                "BeiDou utilization rate: BeiDou satellites used in the fix divided by BeiDou \
                 satellites in view above the elevation mask.",
            ),
            Self::UtilNavic => Some(
                "NavIC utilization rate: NavIC satellites used in the fix divided by NavIC \
                 satellites in view above the elevation mask.",
            ),
            Self::UtilQzss => Some(
                "QZSS utilization rate: QZSS satellites used in the fix divided by QZSS \
                 satellites in view above the elevation mask.",
            ),
            Self::SlipAll => Some(
                "Loss-of-lock (slip) rate per minute, all constellations: how often the receiver \
                 loses a satellite it should still be tracking. A slip is counted when an \
                 above-mask satellite vanishes, or when its SNR drops sharply between epochs. \
                 Averaged over a trailing window. Tune the mask, SNR-drop threshold, and window \
                 in Settings.",
            ),
            Self::SlipGps => Some(
                "GPS loss-of-lock (slip) rate per minute: GPS satellites lost or sharply faded \
                 above the elevation mask, averaged over the slip window.",
            ),
            Self::SlipGlonass => Some(
                "GLONASS loss-of-lock (slip) rate per minute: GLONASS satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            ),
            Self::SlipGalileo => Some(
                "Galileo loss-of-lock (slip) rate per minute: Galileo satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            ),
            Self::SlipBeidou => Some(
                "BeiDou loss-of-lock (slip) rate per minute: BeiDou satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            ),
            Self::SlipNavic => Some(
                "NavIC loss-of-lock (slip) rate per minute: NavIC satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            ),
            Self::SlipQzss => Some(
                "QZSS loss-of-lock (slip) rate per minute: QZSS satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            ),
            _ => None,
        }
    }
}

/// Per-file shade offsets applied to each metric's base colour.
///
/// Keeping hue fixed and only shifting value/lightness preserves metric identity
/// while still making overlapping lines from different files distinguishable.
///
/// Unlike [`gt_ui_theme::track_color`], which cycles a full colour palette per
/// (file, track) for the map (where hue *is* the identity signal), the plot
/// needs hue to stay tied to the metric - so files are distinguished by
/// lightness shift and line style ([`FILE_LINE_STYLES`]) instead.
const FILE_SHADE_FACTORS: [i16; 7] = [0, 22, -22, 12, -12, 32, -32];

/// File-level line styles to keep perfectly overlapping lines distinguishable.
///
/// Color still carries metric identity. Style only disambiguates file source.
const FILE_LINE_STYLES: [LineStyle; 5] = [
    LineStyle::Solid,
    LineStyle::Dashed { length: 6.0 },
    LineStyle::Dotted { spacing: 5.0 },
    LineStyle::Dashed { length: 10.0 },
    LineStyle::Dotted { spacing: 8.0 },
];
/// Grid base-color intensity, as a multiplier on the theme text color.
/// egui_plot's grid stroke width is fixed at 1.0, so with the thinner default
/// data lines the grid at full text-color brightness would dominate; dimming
/// it restores the lines as the visually strongest element.
const GRID_COLOR_STRENGTH: f32 = 0.5;
/// Default stroke width of the metric and channel plot lines.  Slightly below
/// egui_plot's 1.0 default: many lines are enabled by default, and a thinner
/// stroke keeps overlapping lines readable.
pub const DEFAULT_PLOT_LINE_WIDTH: f32 = 0.75;
/// Allowed plot line width, shared by the display-settings slider and the
/// clamp applied to persisted settings on load.
pub const PLOT_LINE_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 0.5..=5.0;
/// Default legend overlay position, anchored just inside the plot's top-left
/// corner.
pub const LEGEND_DOCK_OFFSET: egui::Vec2 = egui::vec2(10.0, 10.0);
/// Sub-pixel tolerance for [`legend_is_docked`]'s "is this exactly the dock
/// position" check - not to be confused with [`LEGEND_DOCK_SNAP_RADIUS`],
/// the much larger radius used to *move* the legend onto the dock position.
const LEGEND_DOCK_POSITION_TOLERANCE: f32 = 1.0;
/// Background opacity of the file-style legend overlay, matching the default
/// `background_alpha` of egui_plot's built-in legend.
const LEGEND_BACKGROUND_ALPHA: f32 = 0.75;
/// Minimum distance the dragged legend keeps from the plot edges.
const LEGEND_EDGE_MARGIN: f32 = 6.0;
/// Distance from the docked top-left position within which a dragged legend
/// snaps back to docking, so dropping it near the corner re-docks it without
/// requiring a click on the re-dock button.
const LEGEND_DOCK_SNAP_RADIUS: f32 = 32.0;
/// Dimensions of the line-style swatch painted next to each legend entry.
const SWATCH_SIZE: egui::Vec2 = egui::vec2(26.0, 10.0);
const SWATCH_STROKE_WIDTH: f32 = 2.0;
/// Gap between dashes as a fraction of the dash length.
const SWATCH_DASH_GAP_RATIO: f32 = 0.62;
const SWATCH_DOT_RADIUS: f32 = 1.7;

/// Overlap budget expressed as a multiple of the single-track target
/// (`≈ 2 × plot_width_px`).  Tracks that overlap in time each span the full
/// width.  This many of them can do so at full resolution before [`budget_cap`]
/// starts sharing the budget between them.  See [`budget_cap`].
const BUDGET_TRACK_MULTIPLE: usize = 8;

fn metric_line_color(kind: MetricKind, file_index: usize, dark_mode: bool) -> Color32 {
    shade_color(
        gt_ui_theme::metric_color(kind, dark_mode),
        file_shade_factor(file_index),
    )
}

/// The channel chip/line palette, cycled by a channel's index in the sorted
/// union of loaded channel names. Channels are dynamic, so unlike the metrics
/// they cannot carry a hardcoded per-variant color; hues are picked to avoid
/// the strong metric colors (velocity yellow, EPH magenta, heading orange).
const CHANNEL_PALETTE: [Color32; 6] = [
    Color32::from_rgb(102, 204, 153), // spring green
    Color32::from_rgb(153, 128, 250), // lavender
    Color32::from_rgb(64, 175, 255),  // azure
    Color32::from_rgb(230, 126, 179), // rose
    Color32::from_rgb(181, 204, 92),  // olive
    Color32::from_rgb(94, 210, 217),  // teal
];

/// The chip color of the `index`-th channel (its position in the sorted name
/// union). The palette cycles past its length.
fn channel_color(index: usize) -> Color32 {
    let palette = CHANNEL_PALETTE;
    palette
        .get(index % palette.len())
        .copied()
        .unwrap_or(Color32::GRAY)
}

fn channel_line_color(index: usize, file_index: usize) -> Color32 {
    shade_color(channel_color(index), file_shade_factor(file_index))
}

/// Hue step between a vector channel's components, as a fraction of the
/// full hue circle. 25 degrees proved too close to tell apart in practice;
/// at 60 degrees x/y/z read as clearly different colors. The chip's bar
/// strip ties the rotated hues back to their channel, so staying near the
/// base hue matters less than being distinct.
const COMPONENT_HUE_STEP: f32 = 60.0 / 360.0;

/// The `component`-th line color of a channel: the channel color with its
/// hue rotated in alternating steps (base, +25, -25, +50, ...), so a vector
/// channel's components separate without leaving its color family.
fn component_color(base: Color32, component: usize) -> Color32 {
    if component == 0 {
        return base;
    }
    let steps = component.div_ceil(2) as f32;
    let sign = if component % 2 == 1 { 1.0 } else { -1.0 };
    let mut hsva = egui::ecolor::Hsva::from(base);
    hsva.h = (hsva.h + sign * steps * COMPONENT_HUE_STEP).rem_euclid(1.0);
    Color32::from(hsva)
}

/// Full-resolution sample target for a single track filling the plot width:
/// ~2 samples per pixel, floored so a very narrow plot still has usable detail.
fn single_target(available_width: f32) -> usize {
    #[expect(
        clippy::cast_sign_loss,
        reason = "available_width is always ≥ 0 in practice; .max(0.0) makes it explicit"
    )]
    let px = available_width.max(0.0) as usize;
    (px * 2).max(400)
}

/// Upper bound on any single track's sample target.
///
/// Tracks that overlap in time all span the full plot width, so without a cap
/// N of them would each request [`single_target`] points.  Sharing a budget of
/// `single × BUDGET_TRACK_MULTIPLE` across the visible tracks bounds the total
/// handed to egui_plot in that worst case.  Tracks that occupy only part of the
/// width get far less via [`track_target`].  This cap only bites when many tracks
/// pile up in the same time range.
fn budget_cap(available_width: f32, visible_count: usize) -> usize {
    let single = single_target(available_width);
    let count = visible_count.max(1);
    (single.saturating_mul(BUDGET_TRACK_MULTIPLE) / count).clamp(2, single)
}

/// Sample target for one track: ~2 points per pixel of the track's *visible*
/// width within the current view, capped by `cap` and floored at 2 (a single
/// segment).
///
/// A track that occupies only a few pixels when zoomed out therefore hands only
/// a few points to the plot.  Paired with the mipmap cascading down to 2 points
/// (so a coarse-enough level actually exists), this is what keeps many short
/// tracks cheap - the fixed per-track target it replaces always pulled hundreds
/// of points per track regardless of on-screen size.
fn track_target(
    x_range: Option<(f64, f64)>,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    cap: usize,
) -> usize {
    let view = x_max - x_min;
    let Some((lo, hi)) = x_range else { return 2 };
    if !view.is_finite() || view <= 0.0 {
        return cap;
    }
    let visible = (hi.min(x_max) - lo.max(x_min)).max(0.0);
    let pixels = f64::from(available_width) * (visible / view);
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "pixels is finite and ≥ 0; truncating a ~2-per-pixel count to an integer is intended"
    )]
    let want = (2.0 * pixels) as usize;
    want.clamp(2, cap)
}

fn file_shade_factor(file_index: usize) -> i16 {
    let idx = file_index % FILE_SHADE_FACTORS.len();
    FILE_SHADE_FACTORS.get(idx).copied().unwrap_or(0)
}

fn file_line_style(file_index: usize) -> LineStyle {
    let idx = file_index % FILE_LINE_STYLES.len();
    FILE_LINE_STYLES
        .get(idx)
        .copied()
        .unwrap_or(LineStyle::Solid)
}

fn paint_line_style_swatch(ui: &mut egui::Ui, style: LineStyle, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(SWATCH_SIZE, egui::Sense::hover());
    let y = rect.center().y;
    let start = egui::pos2(rect.left(), y);
    let end = egui::pos2(rect.right(), y);
    let painter = ui.painter();
    match style {
        LineStyle::Solid => {
            painter.line_segment([start, end], egui::Stroke::new(SWATCH_STROKE_WIDTH, color));
        }
        LineStyle::Dashed { length } => {
            painter.extend(egui::Shape::dashed_line(
                &[start, end],
                egui::Stroke::new(SWATCH_STROKE_WIDTH, color),
                length,
                length * SWATCH_DASH_GAP_RATIO,
            ));
        }
        LineStyle::Dotted { spacing } => {
            painter.extend(egui::Shape::dotted_line(
                &[start, end],
                color,
                spacing,
                SWATCH_DOT_RADIUS,
            ));
        }
    }
    response
}

/// Shifts a color toward white (positive `factor_pct`) or black (negative)
/// by the given percentage.
fn shade_color(color: Color32, factor_pct: i16) -> Color32 {
    let (target, amount_pct) = if factor_pct >= 0 {
        (255, factor_pct)
    } else {
        (0, -factor_pct)
    };
    let num = i32::from(amount_pct.clamp(0, 100));
    Color32::from_rgb(
        gt_ui_theme::lerp_channel(color.r(), target, num, 100),
        gt_ui_theme::lerp_channel(color.g(), target, num, 100),
        gt_ui_theme::lerp_channel(color.b(), target, num, 100),
    )
}

/// Global per-metric visibility flags.
///
/// Disabling a metric hides it for **all** tracks at once, making it easy to
/// declutter the plot without touching per-track settings.
#[derive(Debug, Clone, Copy)]
pub struct MetricVisibility {
    pub sats_seen: bool,
    pub sats_fix: bool,
    pub gps_seen: bool,
    pub gps_fix: bool,
    pub glonass_seen: bool,
    pub glonass_fix: bool,
    pub galileo_seen: bool,
    pub galileo_fix: bool,
    pub beidou_seen: bool,
    pub beidou_fix: bool,
    pub navic_seen: bool,
    pub navic_fix: bool,
    pub qzss_seen: bool,
    pub qzss_fix: bool,
    pub velocity: bool,
    pub eph: bool,
    pub heading_deg: bool,
    pub clock_delta_ms: bool,
    pub util_all: bool,
    pub util_gps: bool,
    pub util_glonass: bool,
    pub util_galileo: bool,
    pub util_beidou: bool,
    pub util_navic: bool,
    pub util_qzss: bool,
    pub slip_all: bool,
    pub slip_gps: bool,
    pub slip_glonass: bool,
    pub slip_galileo: bool,
    pub slip_beidou: bool,
    pub slip_navic: bool,
    pub slip_qzss: bool,
    pub snap_error: bool,
}

impl Default for MetricVisibility {
    fn default() -> Self {
        Self {
            sats_seen: true,
            sats_fix: true,
            gps_seen: true,
            gps_fix: true,
            glonass_seen: true,
            glonass_fix: true,
            galileo_seen: true,
            galileo_fix: true,
            beidou_seen: true,
            beidou_fix: true,
            navic_seen: true,
            navic_fix: true,
            qzss_seen: true,
            qzss_fix: true,
            velocity: true,
            eph: true,
            heading_deg: true,
            clock_delta_ms: true,
            util_all: true,
            util_gps: true,
            util_glonass: true,
            util_galileo: true,
            util_beidou: true,
            util_navic: true,
            util_qzss: true,
            slip_all: true,
            slip_gps: true,
            slip_glonass: true,
            slip_galileo: true,
            slip_beidou: true,
            slip_navic: true,
            slip_qzss: true,
            snap_error: true,
        }
    }
}

impl MetricVisibility {
    /// Returns the current visibility for `kind`.
    pub fn field(&self, kind: MetricKind) -> bool {
        match kind {
            MetricKind::SatsSeen => self.sats_seen,
            MetricKind::SatsFix => self.sats_fix,
            MetricKind::GpsSeen => self.gps_seen,
            MetricKind::GpsFix => self.gps_fix,
            MetricKind::GlonassSeen => self.glonass_seen,
            MetricKind::GlonassFix => self.glonass_fix,
            MetricKind::GalileoSeen => self.galileo_seen,
            MetricKind::GalileoFix => self.galileo_fix,
            MetricKind::BeidouSeen => self.beidou_seen,
            MetricKind::BeidouFix => self.beidou_fix,
            MetricKind::NavicSeen => self.navic_seen,
            MetricKind::NavicFix => self.navic_fix,
            MetricKind::QzssSeen => self.qzss_seen,
            MetricKind::QzssFix => self.qzss_fix,
            MetricKind::Velocity => self.velocity,
            MetricKind::Eph => self.eph,
            MetricKind::HeadingDeg => self.heading_deg,
            MetricKind::ClockDeltaMs => self.clock_delta_ms,
            MetricKind::UtilAll => self.util_all,
            MetricKind::UtilGps => self.util_gps,
            MetricKind::UtilGlonass => self.util_glonass,
            MetricKind::UtilGalileo => self.util_galileo,
            MetricKind::UtilBeidou => self.util_beidou,
            MetricKind::UtilNavic => self.util_navic,
            MetricKind::UtilQzss => self.util_qzss,
            MetricKind::SlipAll => self.slip_all,
            MetricKind::SlipGps => self.slip_gps,
            MetricKind::SlipGlonass => self.slip_glonass,
            MetricKind::SlipGalileo => self.slip_galileo,
            MetricKind::SlipBeidou => self.slip_beidou,
            MetricKind::SlipNavic => self.slip_navic,
            MetricKind::SlipQzss => self.slip_qzss,
            MetricKind::SnapError => self.snap_error,
        }
    }

    /// Returns a mutable reference to the visibility flag for `kind`.
    pub fn field_mut(&mut self, kind: MetricKind) -> &mut bool {
        match kind {
            MetricKind::SatsSeen => &mut self.sats_seen,
            MetricKind::SatsFix => &mut self.sats_fix,
            MetricKind::GpsSeen => &mut self.gps_seen,
            MetricKind::GpsFix => &mut self.gps_fix,
            MetricKind::GlonassSeen => &mut self.glonass_seen,
            MetricKind::GlonassFix => &mut self.glonass_fix,
            MetricKind::GalileoSeen => &mut self.galileo_seen,
            MetricKind::GalileoFix => &mut self.galileo_fix,
            MetricKind::BeidouSeen => &mut self.beidou_seen,
            MetricKind::BeidouFix => &mut self.beidou_fix,
            MetricKind::NavicSeen => &mut self.navic_seen,
            MetricKind::NavicFix => &mut self.navic_fix,
            MetricKind::QzssSeen => &mut self.qzss_seen,
            MetricKind::QzssFix => &mut self.qzss_fix,
            MetricKind::Velocity => &mut self.velocity,
            MetricKind::Eph => &mut self.eph,
            MetricKind::HeadingDeg => &mut self.heading_deg,
            MetricKind::ClockDeltaMs => &mut self.clock_delta_ms,
            MetricKind::UtilAll => &mut self.util_all,
            MetricKind::UtilGps => &mut self.util_gps,
            MetricKind::UtilGlonass => &mut self.util_glonass,
            MetricKind::UtilGalileo => &mut self.util_galileo,
            MetricKind::UtilBeidou => &mut self.util_beidou,
            MetricKind::UtilNavic => &mut self.util_navic,
            MetricKind::UtilQzss => &mut self.util_qzss,
            MetricKind::SlipAll => &mut self.slip_all,
            MetricKind::SlipGps => &mut self.slip_gps,
            MetricKind::SlipGlonass => &mut self.slip_glonass,
            MetricKind::SlipGalileo => &mut self.slip_galileo,
            MetricKind::SlipBeidou => &mut self.slip_beidou,
            MetricKind::SlipNavic => &mut self.slip_navic,
            MetricKind::SlipQzss => &mut self.slip_qzss,
            MetricKind::SnapError => &mut self.snap_error,
        }
    }

    /// Returns `true` when every *currently shown* metric is enabled.  Advanced
    /// metrics are ignored while the advanced section is collapsed (`show_advanced
    /// == false`), and per-constellation metrics whose constellation is absent
    /// from the loaded data are ignored too (their chips are hidden), so the
    /// show/hide-all button neither reflects nor toggles them.
    fn all_enabled(self, present: &HashSet<Constellation>, show_advanced: bool) -> bool {
        MetricKind::iter()
            .filter(|&k| metric_is_shown(k, present, show_advanced))
            .all(|k| self.field(k))
    }

    /// Set every *currently shown* metric to `enabled`, leaving hidden metrics
    /// (collapsed advanced section, or an absent constellation) untouched.
    fn set_all(&mut self, enabled: bool, present: &HashSet<Constellation>, show_advanced: bool) {
        for k in MetricKind::iter().filter(|&k| metric_is_shown(k, present, show_advanced)) {
            *self.field_mut(k) = enabled;
        }
    }
}

/// Whether a metric's chip and line should be shown, given which constellations
/// appear in the loaded data and whether the advanced section is revealed.
///
/// A per-constellation metric is shown only when that constellation is present;
/// an advanced metric only when the advanced section is open.  This is the
/// single gate shared by chip rendering, line drawing, and the show/hide-all
/// logic so they never disagree about what is on screen.
fn metric_is_shown(
    kind: MetricKind,
    present: &HashSet<Constellation>,
    show_advanced: bool,
) -> bool {
    (show_advanced || !kind.is_advanced())
        && kind.constellation().is_none_or(|c| present.contains(&c))
}

/// Global per-channel visibility, keyed by channel name.
///
/// Channels are dynamic per-file names, so this is a map rather than the flat
/// bool fields of [`MetricVisibility`]. A name that was never toggled is
/// visible, matching the persisted-settings convention (missing key = shown).
/// Names persist across loads: an `accel` hidden once stays hidden in the next
/// recording that carries an `accel`.
#[derive(Debug, Clone, Default)]
pub struct ChannelVisibility(HashMap<String, bool>);

impl ChannelVisibility {
    pub fn is_visible(&self, name: &str) -> bool {
        self.0.get(name).copied().unwrap_or(true)
    }

    pub fn set(&mut self, name: &str, visible: bool) {
        self.0.insert(name.to_owned(), visible);
    }

    /// The toggled entries, sorted by name, for persistence and
    /// change-detection snapshots.
    pub fn entries(&self) -> Vec<(String, bool)> {
        let mut entries: Vec<(String, bool)> =
            self.0.iter().map(|(k, &v)| (k.clone(), v)).collect();
        entries.sort();
        entries
    }
}

/// One channel present in the loaded data, unioned across every track's
/// series: its name, unit label, and palette index (the position in the
/// sorted name list). Recomputed per frame - the union is a handful of
/// entries.
struct LoadedChannel {
    name: String,
    unit: Option<String>,
    color_index: usize,
    /// Component count (1 for a scalar channel), for the chip's color bars.
    components: NonZeroUsize,
}

/// The sorted union of channels across all series, with palette indices.
fn loaded_channels<'a>(
    all_channels: impl Iterator<Item = &'a crate::series::ChannelSeries>,
) -> Vec<LoadedChannel> {
    let mut by_name: Vec<(&str, Option<&str>, NonZeroUsize)> = Vec::new();
    for channel in all_channels {
        // `build_channel_series` always emits at least one component; the
        // MIN fallback only satisfies the non-zero type.
        let components = NonZeroUsize::new(channel.components.len()).unwrap_or(NonZeroUsize::MIN);
        match by_name
            .iter_mut()
            .find(|(name, _, _)| *name == channel.name)
        {
            // The widest series wins: files may carry differing component
            // counts under one name, and the chip should show them all.
            Some((_, _, widest)) => *widest = (*widest).max(components),
            None => by_name.push((&channel.name, channel.unit.as_deref(), components)),
        }
    }
    by_name.sort();
    by_name
        .into_iter()
        .enumerate()
        .map(|(color_index, (name, unit, components))| LoadedChannel {
            name: name.to_owned(),
            unit: unit.map(str::to_owned),
            color_index,
            components,
        })
        .collect()
}

/// Which optional chip sections are revealed, gating their lines exactly as
/// the chips are gated.
#[derive(Clone, Copy)]
struct SectionGates {
    show_advanced: bool,
    show_channels: bool,
}

/// A chip the pointer can rest on: a metric's, or a loaded channel's (by
/// name). Drives the hover highlight that dims every other line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HoveredChip {
    Metric(MetricKind),
    Channel(String),
}

/// Cached level selections for every metric of one track's series, plus one
/// per channel component (dynamic, hence no `Copy`).
#[derive(Debug, Clone, Default)]
struct TripLevelCache {
    total_seen: LevelSelection,
    total_fix: LevelSelection,
    gps_seen: LevelSelection,
    gps_fix: LevelSelection,
    glonass_seen: LevelSelection,
    glonass_fix: LevelSelection,
    galileo_seen: LevelSelection,
    galileo_fix: LevelSelection,
    beidou_seen: LevelSelection,
    beidou_fix: LevelSelection,
    navic_seen: LevelSelection,
    navic_fix: LevelSelection,
    qzss_seen: LevelSelection,
    qzss_fix: LevelSelection,
    velocity_kmh: LevelSelection,
    eph_m: LevelSelection,
    heading_deg: LevelSelection,
    clock_delta_ms: LevelSelection,
    util_all: LevelSelection,
    util_gps: LevelSelection,
    util_glonass: LevelSelection,
    util_galileo: LevelSelection,
    util_beidou: LevelSelection,
    util_navic: LevelSelection,
    util_qzss: LevelSelection,
    slip_all: LevelSelection,
    slip_gps: LevelSelection,
    slip_glonass: LevelSelection,
    slip_galileo: LevelSelection,
    slip_beidou: LevelSelection,
    slip_navic: LevelSelection,
    slip_qzss: LevelSelection,
    /// One selection per channel component, mirroring the series' channel
    /// structure (outer: channel, inner: component).
    channels: Vec<Vec<LevelSelection>>,
}

impl TripLevelCache {
    /// `None` for metrics with no mipmap: snap error draws from the external
    /// per-run series, not from `TrackSeries`.
    fn level_for(&self, kind: MetricKind) -> Option<LevelSelection> {
        Some(match kind {
            MetricKind::SatsSeen => self.total_seen,
            MetricKind::SatsFix => self.total_fix,
            MetricKind::GpsSeen => self.gps_seen,
            MetricKind::GpsFix => self.gps_fix,
            MetricKind::GlonassSeen => self.glonass_seen,
            MetricKind::GlonassFix => self.glonass_fix,
            MetricKind::GalileoSeen => self.galileo_seen,
            MetricKind::GalileoFix => self.galileo_fix,
            MetricKind::BeidouSeen => self.beidou_seen,
            MetricKind::BeidouFix => self.beidou_fix,
            MetricKind::NavicSeen => self.navic_seen,
            MetricKind::NavicFix => self.navic_fix,
            MetricKind::QzssSeen => self.qzss_seen,
            MetricKind::QzssFix => self.qzss_fix,
            MetricKind::Velocity => self.velocity_kmh,
            MetricKind::Eph => self.eph_m,
            MetricKind::HeadingDeg => self.heading_deg,
            MetricKind::ClockDeltaMs => self.clock_delta_ms,
            MetricKind::UtilAll => self.util_all,
            MetricKind::UtilGps => self.util_gps,
            MetricKind::UtilGlonass => self.util_glonass,
            MetricKind::UtilGalileo => self.util_galileo,
            MetricKind::UtilBeidou => self.util_beidou,
            MetricKind::UtilNavic => self.util_navic,
            MetricKind::UtilQzss => self.util_qzss,
            MetricKind::SlipAll => self.slip_all,
            MetricKind::SlipGps => self.slip_gps,
            MetricKind::SlipGlonass => self.slip_glonass,
            MetricKind::SlipGalileo => self.slip_galileo,
            MetricKind::SlipBeidou => self.slip_beidou,
            MetricKind::SlipNavic => self.slip_navic,
            MetricKind::SlipQzss => self.slip_qzss,
            MetricKind::SnapError => return None,
        })
    }
}

impl crate::series::TrackSeries {
    /// `None` for metrics with no mipmap, mirroring
    /// [`TripLevelCache::level_for`].
    fn mipmap_for(&self, kind: MetricKind) -> Option<&gt_egui_mipmap::MipMap> {
        Some(match kind {
            MetricKind::SatsSeen => &self.total_seen,
            MetricKind::SatsFix => &self.total_fix,
            MetricKind::GpsSeen => &self.gps_seen,
            MetricKind::GpsFix => &self.gps_fix,
            MetricKind::GlonassSeen => &self.glonass_seen,
            MetricKind::GlonassFix => &self.glonass_fix,
            MetricKind::GalileoSeen => &self.galileo_seen,
            MetricKind::GalileoFix => &self.galileo_fix,
            MetricKind::BeidouSeen => &self.beidou_seen,
            MetricKind::BeidouFix => &self.beidou_fix,
            MetricKind::NavicSeen => &self.navic_seen,
            MetricKind::NavicFix => &self.navic_fix,
            MetricKind::QzssSeen => &self.qzss_seen,
            MetricKind::QzssFix => &self.qzss_fix,
            MetricKind::Velocity => &self.velocity_kmh,
            MetricKind::Eph => &self.eph_m,
            MetricKind::HeadingDeg => &self.heading_deg,
            MetricKind::ClockDeltaMs => &self.clock_delta_ms,
            MetricKind::UtilAll => &self.util_all,
            MetricKind::UtilGps => &self.util_gps,
            MetricKind::UtilGlonass => &self.util_glonass,
            MetricKind::UtilGalileo => &self.util_galileo,
            MetricKind::UtilBeidou => &self.util_beidou,
            MetricKind::UtilNavic => &self.util_navic,
            MetricKind::UtilQzss => &self.util_qzss,
            MetricKind::SlipAll => &self.slip_all,
            MetricKind::SlipGps => &self.slip_gps,
            MetricKind::SlipGlonass => &self.slip_glonass,
            MetricKind::SlipGalileo => &self.slip_galileo,
            MetricKind::SlipBeidou => &self.slip_beidou,
            MetricKind::SlipNavic => &self.slip_navic,
            MetricKind::SlipQzss => &self.slip_qzss,
            MetricKind::SnapError => return None,
        })
    }
}

/// Persistent state for the track plot panel.
///
/// Plot panel visibility is managed externally (via the tiles tree in the app),
/// so this struct only tracks the cursor hover time and the mipmap series cache.
#[derive(Debug, Clone)]
pub struct PlotState {
    /// Time currently hovered by the plot cursor, written each frame.
    /// `None` when the cursor is outside the plot area.
    pub hovered_time: Option<DateTime<Utc>>,
    /// Global per-metric visibility - toggled via the chip row above the plot.
    pub metric_vis: MetricVisibility,
    /// Whether the plot grid lines are visible.
    pub show_grid: bool,
    /// Stroke width of the metric and channel lines, adjusted via the plot
    /// display popup (the gear button in the chip row).
    pub line_width: f32,
    /// When true, the plot x-range tracks the map viewport.
    pub sync_to_map: bool,
    /// Whether to draw the masked-satellite anomaly markers (a used satellite
    /// below the elevation mask).  Toggled in Settings.
    pub mark_masked_fix: bool,
    /// Whether the advanced analysis chips (satellite utilization) are revealed.
    /// Off by default - these metrics are hidden until the user opts in.
    pub show_advanced_metrics: bool,
    /// Whether the ad-hoc channel chips and lines are revealed. Off by
    /// default, like the advanced section; the toggle only renders when a
    /// loaded track carries channels.
    pub show_channels: bool,
    /// Global per-channel visibility - toggled via the channel chips.
    pub channel_vis: ChannelVisibility,
    /// Analysis parameters the cached series were built with (elevation mask).
    /// Changing it via [`PlotState::set_analysis`] re-derives the affected series.
    pub analysis: AnalysisConfig,
    /// Whether the file-style legend body is collapsed.
    pub file_legend_collapsed: bool,
    /// Legend overlay position offset from the plot's top-left corner.
    pub file_legend_offset: egui::Vec2,
    /// Legend overlay size measured on the previous frame.
    /// Used to size this frame's drag-sensing background before the
    /// legend's content is laid out.
    file_legend_size: egui::Vec2,
    /// File index currently hovered in the legend overlay.
    pub legend_hover_file: Option<usize>,
    /// Mipmap cascade for every track in every loaded file.
    pub(crate) series_cache: Vec<TrackSeries>,
    /// Cached level selections, one entry per series.
    /// Invalidated when the effective plot bounds or target sample count changes.
    level_cache: Vec<TripLevelCache>,
    /// The `(eff_x_min, eff_x_max, plot_width_bits, sample_cap)` at which the
    /// current `level_cache` was computed.  `None` forces a recompute on the
    /// next frame.  Used for hysteresis: the cache is reused as long as the view
    /// has not moved by more than ~10 pixels and neither the plot width nor the
    /// per-track sample cap has changed since the last recompute.
    last_computed_bounds: Option<(f64, f64, u32, usize)>,
    /// The map x-range (encoded as bit-pattern pairs) most recently applied to
    /// the plot via `set_plot_bounds_x`.  Used to detect changes and avoid
    /// re-applying the same range every frame (which would prevent manual zoom).
    applied_map_x_range: Option<(u64, u64)>,
    /// Per-track snap error mipmaps and marker lists, rebuilt only when a
    /// track's series `Arc` changes (see [`sync_snap_error_cache`]).
    snap_error_cache: HashMap<TrackRef, SnapErrorPlotCache>,
    /// Whether the plot cursor was snapped close to a data point on the most
    /// recently rendered frame.
    ///
    /// Set by [`show_track_plot`] using a 2-D screen-space distance check
    /// against every enabled metric line.  The app layer forwards this to
    /// [`gt_ui_types::MapHighlight::plot_hover_snapped`] so the map overlay
    /// activates only when the cursor is genuinely near a plotted value, not
    /// just anywhere inside the plot area.
    pub plot_cursor_snapped: bool,
}

impl Default for PlotState {
    fn default() -> Self {
        Self {
            hovered_time: None,
            metric_vis: MetricVisibility::default(),
            show_grid: true,
            line_width: DEFAULT_PLOT_LINE_WIDTH,
            sync_to_map: true,
            mark_masked_fix: true,
            show_advanced_metrics: false,
            show_channels: false,
            channel_vis: ChannelVisibility::default(),
            analysis: AnalysisConfig::default(),
            file_legend_collapsed: false,
            file_legend_offset: LEGEND_DOCK_OFFSET,
            file_legend_size: egui::Vec2::ZERO,
            legend_hover_file: None,
            series_cache: Vec::new(),
            level_cache: Vec::new(),
            last_computed_bounds: None,
            applied_map_x_range: None,
            snap_error_cache: HashMap::new(),
            plot_cursor_snapped: false,
        }
    }
}

impl PlotState {
    /// Incorporate pre-built mipmap series for file `fi`.
    ///
    /// Replaces any existing series for that file index and invalidates the
    /// level cache so the next frame recomputes level selections.
    pub fn integrate_file(&mut self, fi: usize, prepared: crate::PreparedSeries) {
        self.series_cache.retain(|s| s.fi != fi);
        // Re-insert at a stable position: find where fi would sit among existing
        // file indices so the ordering stays consistent with the loaded_files vec.
        let insert_pos = self
            .series_cache
            .iter()
            .position(|s| s.fi > fi)
            .unwrap_or(self.series_cache.len());
        for (offset, mut series) in prepared.0.into_iter().enumerate() {
            series.fi = fi;
            self.series_cache.insert(insert_pos + offset, series);
        }
        self.invalidate_level_cache();
    }

    /// Rebuild series for all currently loaded files from scratch.
    ///
    /// Called after file deletion - runs on the UI thread since deletion is
    /// cheap (files already parsed, just re-indexing the surviving files).
    pub fn rebuild_all(&mut self, files: &[LoadedFile]) {
        self.series_cache = build_all_series(files, self.analysis);
        self.invalidate_level_cache();
    }

    /// Apply new analysis parameters (elevation mask), re-deriving only the
    /// mask-dependent series in place.  A no-op when `analysis` is unchanged, so
    /// it is cheap to call every frame while a settings control is open.
    pub fn set_analysis(&mut self, files: &[LoadedFile], analysis: AnalysisConfig) {
        if self.analysis == analysis {
            return;
        }
        self.analysis = analysis;
        for series in &mut self.series_cache {
            if let Some(track) = files
                .get(series.fi)
                .and_then(|file| file.tracks.get(series.ti))
            {
                series.apply_analysis(track, analysis);
            }
        }
        self.invalidate_level_cache();
    }

    fn invalidate_level_cache(&mut self) {
        self.last_computed_bounds = None;
        self.level_cache.clear();
    }
}

/// Render the track plot panel.
///
/// - `map_hover_time`: the timestamp of the TPV point currently hovered on
///   the map (if any).  The plot draws a vertical cursor line at this time so
///   the user can see the map-selected moment in context.
/// - `state.hovered_time` is written with the plot cursor's current time each
///   frame.  The caller should forward this to `MapHighlight::plot_hover_time`
///   before drawing the map so the renderer can cross-highlight the nearest
///   TPV arrow.
#[expect(
    clippy::too_many_arguments,
    reason = "plot rendering needs files, visibility/filter state, cross-highlight inputs, map-sync range, and mutable plot state"
)]
pub fn show_track_plot(
    ui: &mut egui::Ui,
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    hover_scope: Option<HighlightScope>,
    map_hover_time: Option<DateTime<Utc>>,
    // Time span of the match hovered in the query results table, drawn as a
    // shaded band beneath the series so the match shows in metric context.
    match_hover_time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    // When map→plot sync is enabled, this carries the Unix-second x range
    // computed from TPV points visible in the current map viewport.
    // The plot will pan/zoom to this range the first frame it changes.
    map_sync_x_range: Option<(f64, f64)>,
    // Snap error per track, resolved by the app from completed snap runs
    // (see `gt_ui_types::SnapErrorSeries`).
    snap_error: &SnapErrorSeries,
    state: &mut PlotState,
) {
    // Compute the per-series visibility mask once so the three downstream
    // consumers - visible_count, the full-x-range loop, and the render loop -
    // all share a single pass instead of calling trip_is_visible three times
    // per series per frame.
    let visible: Vec<bool> = state
        .series_cache
        .iter()
        .map(|s| trip_is_visible(visibility, filter, files, s.fi, s.ti))
        .collect();
    let visible_count = visible.iter().filter(|&&v| v).count();

    if visible_count == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new("Load a .gtd file to see track metrics")
                    .weak()
                    .italics(),
            );
        });
        state.hovered_time = None;
        return;
    }

    // Per-series count, for the line-name prefix.  Distinct from the
    // per-file count below, which gates the file legend overlay.
    let multi_track = visible_count > 1;
    let visible_files: Vec<usize> = {
        let mut file_indices = BTreeSet::new();
        for (series, &is_vis) in state.series_cache.iter().zip(visible.iter()) {
            if is_vis {
                file_indices.insert(series.fi);
            }
        }
        file_indices.into_iter().collect()
    };

    // Constellations present anywhere in the loaded data.  Per-constellation
    // chips and lines are gated on this so a constellation with no data (e.g.
    // NavIC or QZSS in a GPS-only recording) never clutters the UI.
    let present: HashSet<Constellation> = state
        .series_cache
        .iter()
        .flat_map(|s| s.present.iter().copied())
        .collect();

    // Channels present anywhere in the loaded data, unioned like the
    // constellations: the Channels toggle and chips render only when a track
    // actually carries channels.
    let channels = loaded_channels(state.series_cache.iter().flat_map(|s| s.channels.iter()));

    // Whether any visible track has a completed snap run: gates the snap
    // error chip (disabled with hover text until a run completes) and the
    // per-point hover hit-testing.
    let snap_error_available = snap_error_available(&state.series_cache, &visible, snap_error);

    // Draw the per-metric filter row before the plot so it consumes vertical
    // space first.  `ui.available_height()` below then gives the remainder.
    let hovered_chip = metric_filter_row(
        ui,
        &mut state.metric_vis,
        &present,
        &channels,
        &mut state.channel_vis,
        &mut state.show_grid,
        &mut state.line_width,
        &mut state.sync_to_map,
        &mut state.show_advanced_metrics,
        &mut state.show_channels,
        snap_error_available,
    );

    // Sample budgeting: each track requests ~2 points per pixel of its *visible*
    // width (`track_target`, computed per series below), so a track that only
    // occupies a few pixels when zoomed out hands over only a few points.  The
    // cap bounds the worst case where many tracks overlap in the same time range
    // and each spans the full width.  Together with the mipmap now cascading
    // down to 2 points, this is what keeps a screen full of short tracks cheap.
    let available_width = ui.available_width();
    let sample_cap = budget_cap(available_width, visible_count);

    // Filter time range → x-axis bounds for mipmap slice clamping.
    let filter_x_min: Option<f64> = filter.time_start.map(|t| t.timestamp() as f64);
    let filter_x_max: Option<f64> = filter.time_end.map(|t| t.timestamp() as f64);

    // Format an x-axis tick label from a Unix timestamp.
    let x_fmt = |mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>| {
        let ts = mark.value as i64;
        DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default()
    };

    // Format the hover tooltip: show time + value.
    let label_fmt = |name: &str, val: &egui_plot::PlotPoint| {
        let ts = val.x as i64;
        let time_str = DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        format!("{name}\n{time_str}\n{:.2}", val.y)
    };

    // Compute the full x range across all visible series so that double-click
    // (which triggers auto-bounds reset) zooms to fit the complete dataset.
    // Uses the precomputed `TrackSeries::x_range` field - O(1) per series.
    let mut full_x_min = f64::INFINITY;
    let mut full_x_max = f64::NEG_INFINITY;
    for (series, &is_vis) in state.series_cache.iter().zip(visible.iter()) {
        if !is_vis {
            continue;
        }

        if let Some((lo, hi)) = series.x_range {
            full_x_min = full_x_min.min(lo);
            full_x_max = full_x_max.max(hi);
        }
    }
    let has_full_range = full_x_min.is_finite() && full_x_max.is_finite();

    sync_snap_error_cache(&mut state.snap_error_cache, snap_error);

    // Split borrows: extract immutable refs to the caches and metric visibility
    // before the closure so the borrow checker can see they are disjoint from
    // the mutable fields written after the closure (`hovered_time`, `level_cache`,
    // `last_computed_bounds`).
    let series_cache = &state.series_cache;
    let snap_error_cache = &state.snap_error_cache;
    let level_cache = &state.level_cache;
    let last_computed_bounds = state.last_computed_bounds;
    let metric_vis = &state.metric_vis;
    let channel_vis = &state.channel_vis;
    let show_channels = state.show_channels;
    let line_width = state.line_width;
    // Anomaly markers ride on the "Util all" line, so they show only when that
    // metric is visible and the settings toggle is on.
    let show_advanced = state.show_advanced_metrics;
    let show_anomalies =
        show_advanced && state.mark_masked_fix && state.metric_vis.field(MetricKind::UtilAll);
    let effective_hover_scope = state
        .legend_hover_file
        .map(|fi| HighlightScope::File {
            file_index: FileIdx::new(fi),
        })
        .or(hover_scope);

    // Encode the incoming map sync range as bit patterns so we can compare
    // without float equality warnings.
    let map_x_key = map_sync_x_range.map(|(a, b)| (a.to_bits(), b.to_bits()));
    let need_map_sync = map_x_key.is_some_and(|k| state.applied_map_x_range != Some(k));

    let mut new_hovered_time: Option<DateTime<Utc>> = None;
    let mut new_computed_bounds: Option<(f64, f64, u32, usize)> = None;
    let mut new_level_cache: Option<Vec<TripLevelCache>> = None;
    let mut new_applied_map_x_range: Option<Option<(u64, u64)>> = None;
    // Nearest masked-satellite anomaly marker under the pointer, with its
    // screen-space distance, resolved across all visible series inside the plot
    // closure and turned into a tooltip after it returns.
    let mut hovered_anomaly: Option<(f32, AnomalyHover)> = None;
    // Nearest snap error point under the pointer, same mechanism.
    let mut hovered_snap: Option<(f32, SnapErrorHover)> = None;
    let show_snap_error = snap_error_available && state.metric_vis.field(MetricKind::SnapError);

    let mut plot = egui_plot::Plot::new("track_plot")
        .height(ui.available_height())
        .show_grid(state.show_grid)
        .grid_color(
            ui.visuals()
                .text_color()
                .gamma_multiply(GRID_COLOR_STRENGTH),
        )
        .x_axis_formatter(x_fmt)
        .label_formatter(label_fmt);

    // Tell egui_plot the full data extent so double-click reset zooms to fit.
    if has_full_range {
        plot = plot.include_x(full_x_min).include_x(full_x_max);
    }

    let dark_mode = ui.visuals().dark_mode;
    // On a light theme, give the plot a faint-grey canvas rather than egui's
    // pure white so the deepened light-variant series lines keep a little
    // separation from the background. Scoped to this plot: restored right after.
    let saved_extreme_bg = ui.visuals().extreme_bg_color;
    if !dark_mode {
        ui.visuals_mut().extreme_bg_color = gt_ui_theme::PLOT_CANVAS_LIGHT;
    }
    let plot_response = plot.show(ui, |plot_ui| {
        let bounds = plot_ui.plot_bounds();
        let plot_x_min = bounds.min()[0];
        let plot_x_max = bounds.max()[0];

        // Intersect the visible plot range with the active time filter.
        //
        // `eff_x_min`/`eff_x_max` may end up inverted when the active filter
        // and the visible viewport don't overlap.  `MipMap` normalizes that
        // into an empty-range query (see `select_level_bounds`).
        let eff_x_min = filter_x_min.map_or(plot_x_min, |f| plot_x_min.max(f));
        let eff_x_max = filter_x_max.map_or(plot_x_max, |f| plot_x_max.min(f));

        // Hysteresis: skip recompute when the view has moved less than ~10 px
        // since the last cache fill.  Converting to data space:
        //   10 px × (data_range / plot_width_px) = 20 × data_range / single
        // (single ≈ 2 × plot_width_px, always ≥ 400).  The cache also depends on
        // the plot width and visible count (both feed the per-track targets), so
        // those are part of the validity check, not just the view bounds.
        let single = single_target(available_width);
        let threshold = 20.0 * (eff_x_max - eff_x_min) / single as f64;
        let cache_valid = last_computed_bounds.is_some_and(|(lx_min, lx_max, lw, lcap)| {
            lw == available_width.to_bits()
                && lcap == sample_cap
                && level_cache.len() == series_cache.len()
                && (eff_x_min - lx_min).abs() <= threshold
                && (eff_x_max - lx_max).abs() <= threshold
        });

        // Recompute if the view changed enough since the last frame.
        // Uses rayon to parallelise across series - each is independent.
        let resolved: std::borrow::Cow<[TripLevelCache]> = if cache_valid {
            std::borrow::Cow::Borrowed(level_cache)
        } else {
            let fresh: Vec<TripLevelCache> = series_cache
                .par_iter()
                .map(|s| compute_level_cache(s, eff_x_min, eff_x_max, available_width, sample_cap))
                .collect();
            new_computed_bounds =
                Some((eff_x_min, eff_x_max, available_width.to_bits(), sample_cap));
            std::borrow::Cow::Owned(fresh)
        };

        // Pan the plot to the map-visible time range when it changes.
        if need_map_sync {
            if let Some((x_min, x_max)) = map_sync_x_range {
                plot_ui.set_plot_bounds_x(x_min..=x_max);
            }
            new_applied_map_x_range = Some(map_x_key);
        }

        // Pointer position for anomaly-marker hit-testing, sampled once.
        let anomaly_pointer = if show_anomalies {
            plot_ui.response().hover_pos()
        } else {
            None
        };
        let snap_pointer = if show_snap_error {
            plot_ui.response().hover_pos()
        } else {
            None
        };

        // The hovered match's time band, before the series so the lines stay
        // on top. A `Span` rather than a polygon: it fills the plot's full
        // height on its own and contributes nothing to the auto-bounds, so
        // the view never re-fits to the band (a polygon sized to the current
        // bounds fed back into the next frame's bounds and made the plot
        // oscillate). A single-point match has no width; a cursor line marks
        // it.
        if let Some((start, end)) = match_hover_time_range {
            let (x0, x1) = (start.timestamp() as f64, end.timestamp() as f64);
            if x0 < x1 {
                // Unnamed: the span draws its name inside the band, and the
                // highlight needs no caption.
                plot_ui.span(Span::new("", x0..=x1).fill(gt_ui_theme::HIGHLIGHT_BLUE_BAND));
            } else {
                plot_ui.vline(
                    VLine::new("Hovered match", x0)
                        .color(gt_ui_theme::HIGHLIGHT_BLUE_SEEK)
                        .width(1.5),
                );
            }
        }

        debug_assert_eq!(visible.len(), series_cache.len());
        for (si, (vis, series)) in visible.iter().zip(series_cache.iter()).enumerate() {
            if !vis {
                continue;
            }
            // `resolved` has the same length as `series_cache` by construction.
            let Some(cache) = resolved.get(si) else {
                continue;
            };
            add_series_lines(
                plot_ui,
                series,
                multi_track,
                cache,
                metric_vis,
                channel_vis,
                &present,
                &channels,
                hovered_chip.as_ref(),
                effective_hover_scope,
                SectionGates {
                    show_advanced,
                    show_channels,
                },
                line_width,
                dark_mode,
                snap_error_cache.get(&series_track_ref(series)),
                SnapErrorViewport {
                    x_min: eff_x_min,
                    x_max: eff_x_max,
                    width: available_width,
                    cap: sample_cap,
                },
                snap_pointer,
                &mut hovered_snap,
            );
            if show_anomalies {
                add_util_anomalies(
                    plot_ui,
                    series,
                    multi_track,
                    anomaly_pointer,
                    &mut hovered_anomaly,
                    dark_mode,
                );
            }
        }

        if let std::borrow::Cow::Owned(owned) = resolved {
            new_level_cache = Some(owned);
        }
        if let Some(t) = map_hover_time {
            let x = t.timestamp() as f64;
            plot_ui.vline(
                VLine::new("Map position", x)
                    .color(gt_ui_theme::HIGHLIGHT_BLUE_SEEK)
                    .width(1.5),
            );
        }

        new_hovered_time = plot_ui
            .pointer_coordinate()
            .and_then(|c| DateTime::from_timestamp(c.x as i64, 0));
    });
    if !dark_mode {
        ui.visuals_mut().extreme_bg_color = saved_extreme_bg;
    }

    // Persist the newly computed level cache (only when a recompute happened).
    if let Some(bounds) = new_computed_bounds {
        state.last_computed_bounds = Some(bounds);
    }
    if let Some(cache) = new_level_cache {
        state.level_cache = cache;
    }
    if let Some(applied) = new_applied_map_x_range {
        state.applied_map_x_range = applied;
    }
    // `hovered_plot_item` is set by egui_plot when the cursor is within its own
    // interact radius of a plotted item, the exact condition that causes
    // egui_plot to show a hover label.  Use it directly so the map overlay
    // activates at precisely the same moment, with no custom approximation.
    state.plot_cursor_snapped =
        plot_response.response.hovered() && plot_response.hovered_plot_item.is_some();

    show_nearest_point_tooltips(ui, &plot_response.response, hovered_anomaly, hovered_snap);

    state.legend_hover_file = show_file_legend_overlay(
        ui,
        files,
        &visible_files,
        plot_response.response.rect,
        state,
    );

    // Clear the hovered time when the cursor leaves the plot area.
    state.hovered_time = if plot_response.response.hovered() {
        new_hovered_time
    } else {
        None
    };
}

fn show_file_legend_overlay(
    ui: &egui::Ui,
    files: &[LoadedFile],
    visible_files: &[usize],
    plot_rect: egui::Rect,
    state: &mut PlotState,
) -> Option<usize> {
    if visible_files.len() < 2 {
        return None;
    }

    let mut redock_requested = false;
    let show_redock_icon = !legend_is_docked(state.file_legend_offset);
    let legend_id = ui.id().with("plot_file_legend_overlay");
    let drag_bg_size = state.file_legend_size;
    let area = Area::new(legend_id)
        .order(egui::Order::Foreground)
        .movable(false)
        .current_pos(plot_rect.min + state.file_legend_offset)
        .show(ui.ctx(), |ui| {
            // Selectable labels also sense drag (for text selection) and
            // would win the hit-test over `drag_response` below.
            ui.style_mut().interaction.selectable_labels = false;

            // Drag-sense the whole body first (bottom z-order) so buttons
            // on top still get their clicks.
            let drag_rect = egui::Rect::from_min_size(ui.cursor().min, drag_bg_size);
            let drag_response =
                ui.interact(drag_rect, legend_id.with("drag_bg"), egui::Sense::drag());

            let hovered_file = Frame::default()
                .inner_margin(egui::Margin::symmetric(8, 4))
                .corner_radius(ui.visuals().window_corner_radius)
                .fill(ui.visuals().extreme_bg_color)
                .stroke(ui.visuals().window_stroke())
                .multiply_with_opacity(LEGEND_BACKGROUND_ALPHA)
                .show(ui, |ui| {
                    let mut hovered_file = None;
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let dock_btn_size = egui::vec2(
                                ui.spacing().interact_size.y,
                                ui.spacing().interact_size.y,
                            );
                            if show_redock_icon
                                && ui
                                    .add_sized(dock_btn_size, Button::new(ICON_ARROW_LINE_UP_LEFT))
                                    .on_hover_text("Re-dock legend to top-left")
                                    .clicked()
                            {
                                redock_requested = true;
                            }
                            ui.add_sized(
                                dock_btn_size,
                                Label::new(RichText::new(ICON_DOTS_SIX).weak()),
                            )
                            .on_hover_cursor(egui::CursorIcon::Grab)
                            .on_hover_text("Drag to move legend");
                            let fold_icon = if state.file_legend_collapsed {
                                ICON_CARET_RIGHT
                            } else {
                                ICON_CARET_DOWN
                            };
                            if ui
                                .small_button(fold_icon)
                                .on_hover_text(if state.file_legend_collapsed {
                                    "Expand legend"
                                } else {
                                    "Collapse legend"
                                })
                                .clicked()
                            {
                                state.file_legend_collapsed = !state.file_legend_collapsed;
                            }
                        });
                        if !state.file_legend_collapsed {
                            for &fi in visible_files {
                                let row = ui.horizontal(|ui| {
                                    let style = file_line_style(fi);
                                    let file_name = files
                                        .get(fi)
                                        .map_or("Unknown file", |f| f.metadata.filename.as_str());
                                    let swatch = paint_line_style_swatch(
                                        ui,
                                        style,
                                        ui.visuals().text_color(),
                                    );
                                    let name = ui.label(RichText::new(file_name).small());
                                    swatch.hovered() || name.hovered()
                                });
                                if row.response.hovered() || row.inner {
                                    hovered_file = Some(fi);
                                }
                            }
                        }
                    });
                    hovered_file
                })
                .inner;

            (hovered_file, drag_response)
        });

    let (hovered_file, drag_response) = area.inner;
    state.file_legend_size = area.response.rect.size();

    if drag_response.dragged() {
        state.file_legend_offset += ui.ctx().input(|i| i.pointer.delta());
    }

    state.file_legend_offset = resolve_legend_offset(
        state.file_legend_offset,
        state.file_legend_size,
        plot_rect,
        redock_requested,
        drag_response.drag_stopped(),
    );
    hovered_file
}

/// Clamps `offset` to the plot's edges, then snaps it to [`LEGEND_DOCK_OFFSET`]
/// if redocking was requested or the drag just ended near that corner.
fn resolve_legend_offset(
    offset: egui::Vec2,
    legend_size: egui::Vec2,
    plot_rect: egui::Rect,
    redock_requested: bool,
    drag_released: bool,
) -> egui::Vec2 {
    let max_x = (plot_rect.width() - legend_size.x - LEGEND_EDGE_MARGIN).max(LEGEND_EDGE_MARGIN);
    let max_y = (plot_rect.height() - legend_size.y - LEGEND_EDGE_MARGIN).max(LEGEND_EDGE_MARGIN);
    let clamped = egui::vec2(
        offset.x.clamp(LEGEND_EDGE_MARGIN, max_x),
        offset.y.clamp(LEGEND_EDGE_MARGIN, max_y),
    );
    // Snap only on release, so dragging away from the dock isn't pulled
    // straight back mid-drag.
    let near_dock =
        drag_released && (clamped - LEGEND_DOCK_OFFSET).length() < LEGEND_DOCK_SNAP_RADIUS;
    if redock_requested || near_dock {
        LEGEND_DOCK_OFFSET
    } else {
        clamped
    }
}

/// Whether `offset` is close enough to [`LEGEND_DOCK_OFFSET`] to be considered docked.
pub fn legend_is_docked(offset: egui::Vec2) -> bool {
    (offset.x - LEGEND_DOCK_OFFSET.x).abs() < LEGEND_DOCK_POSITION_TOLERANCE
        && (offset.y - LEGEND_DOCK_OFFSET.y).abs() < LEGEND_DOCK_POSITION_TOLERANCE
}

/// Render one separator-delimited group of metric chips, folding any
/// "show only this" choice into `show_only` and the hovered metric into `hovered`.
#[expect(
    clippy::too_many_arguments,
    reason = "chip rendering needs the visibility set, both gating inputs, and both fold-out results"
)]
fn chip_group(
    ui: &mut egui::Ui,
    vis: &mut MetricVisibility,
    present: &HashSet<Constellation>,
    kinds: &[MetricKind],
    show_advanced: bool,
    snap_error_available: bool,
    show_only: &mut Option<MetricKind>,
    hovered: &mut Option<HoveredChip>,
) {
    // Skip the whole group - including its leading divider - when none of its
    // chips are shown (e.g. a per-constellation group with no matching data),
    // so the chip row never carries a dangling separator.
    let shown: Vec<MetricKind> = kinds
        .iter()
        .copied()
        .filter(|&k| metric_is_shown(k, present, show_advanced))
        .collect();
    if shown.is_empty() {
        return;
    }
    ui.separator();
    let dark_mode = ui.visuals().dark_mode;
    for kind in shown {
        // The snap error chip stays visible but disabled until a visible
        // track has a completed run - never hidden, per DESIGN.md.
        if kind == MetricKind::SnapError && !snap_error_available {
            disabled_metric_chip(
                ui,
                kind.label(),
                gt_ui_theme::metric_color(kind, dark_mode),
                "No completed snap run for the visible tracks - run snap to road from the \
                 side panel first",
            );
            continue;
        }
        let (s, h) = metric_chip(
            ui,
            vis.field_mut(kind),
            kind.label(),
            gt_ui_theme::metric_color(kind, dark_mode),
            kind.hover_text(),
        );
        if s {
            *show_only = Some(kind);
        }
        if h {
            *hovered = Some(HoveredChip::Metric(kind));
        }
    }
}

/// The two section-reveal toggles: Advanced (always) and Channels (only when
/// a loaded track actually carries channels - with none there is nothing the
/// toggle could reveal). Both sections are hidden by default.
fn section_toggles(
    ui: &mut egui::Ui,
    show_advanced: &mut bool,
    show_channels: &mut bool,
    has_channels: bool,
) {
    if ui
        .selectable_label(*show_advanced, format!("{ICON_GAUGE} Advanced"))
        .on_hover_text(if *show_advanced {
            "Hide advanced metrics"
        } else {
            "Show advanced metrics (satellite utilization and loss-of-lock slip rate)"
        })
        .clicked()
    {
        *show_advanced = !*show_advanced;
    }

    if has_channels
        && ui
            .selectable_label(*show_channels, format!("{ICON_WAVE_SINE} Channels"))
            .on_hover_text(if *show_channels {
                "Hide sensor channels"
            } else {
                "Show sensor channels recorded alongside the track"
            })
            .clicked()
    {
        *show_channels = !*show_channels;
    }
}

/// Render the channel chip group, folding any "show only this" choice into
/// `show_only` and the hovered channel into `hovered`. The channel sibling
/// of [`chip_group`].
fn channel_chip_group(
    ui: &mut egui::Ui,
    channels: &[LoadedChannel],
    channel_vis: &mut ChannelVisibility,
    show_only: &mut Option<String>,
    hovered: &mut Option<HoveredChip>,
) {
    ui.separator();
    for channel in channels {
        let mut enabled = channel_vis.is_visible(&channel.name);
        let label = match &channel.unit {
            Some(unit) => format!("{} ({unit})", channel.name),
            None => channel.name.clone(),
        };
        let (chip_show_only, chip_hovered) = channel_chip(
            ui,
            &mut enabled,
            &label,
            channel_color(channel.color_index),
            channel.components,
            Some("Sensor channel recorded alongside the track"),
        );
        channel_vis.set(&channel.name, enabled);
        if chip_show_only {
            *show_only = Some(channel.name.clone());
        }
        if chip_hovered {
            *hovered = Some(HoveredChip::Channel(channel.name.clone()));
        }
    }
}

/// Draw the per-metric filter controls above the track plot.
///
/// All controls and metric chips share a single `horizontal_wrapped` row so they
/// fill available horizontal space before wrapping - no fixed-height satellite
/// group that forces other chips below it.
///
/// Returns the chip currently being hovered (a metric's or a channel's), or
/// `None`. The caller passes this to `add_series_lines` to highlight the
/// hovered line and dim the rest, mirroring the standard egui-plot legend
/// hover behaviour.
#[expect(
    clippy::too_many_arguments,
    reason = "the filter row owns every plot toggle: grid/sync, the metric and channel visibility sets, and both section gates"
)]
fn metric_filter_row(
    ui: &mut egui::Ui,
    vis: &mut MetricVisibility,
    present: &HashSet<Constellation>,
    channels: &[LoadedChannel],
    channel_vis: &mut ChannelVisibility,
    show_grid: &mut bool,
    line_width: &mut f32,
    sync_to_map: &mut bool,
    show_advanced: &mut bool,
    show_channels: &mut bool,
    snap_error_available: bool,
) -> Option<HoveredChip> {
    // The show/hide-all button and its eye icon track only the currently shown
    // chips, so they ignore advanced metrics while that section is collapsed and
    // per-constellation metrics whose constellation is absent from the data.
    let all_on = vis.all_enabled(present, *show_advanced);
    let mut show_only = None;
    let mut show_only_channel: Option<String> = None;
    let mut hovered_chip = None;

    ui.horizontal_wrapped(|ui| {
        // Sync toggle.
        if ui
            .selectable_label(*sync_to_map, ICON_LINK)
            .on_hover_text(if *sync_to_map {
                "Syncing plot time range to map viewport — click to disable"
            } else {
                "Sync plot time range to map viewport"
            })
            .clicked()
        {
            *sync_to_map = !*sync_to_map;
        }

        // Display settings popup: appearance knobs that are set once and left
        // alone, kept out of the row itself so it stays uncluttered.
        ui.menu_button(ICON_GEAR, |ui| {
            plot_display_menu(ui, show_grid, line_width);
        })
        .response
        .on_hover_text("Plot display settings");

        // Show/hide all (currently shown metrics only).
        let eye_icon = if all_on { ICON_EYE_SLASH } else { ICON_EYE };
        if ui
            .small_button(eye_icon)
            .on_hover_text(if all_on {
                "Hide all metrics"
            } else {
                "Show all metrics"
            })
            .clicked()
        {
            vis.set_all(!all_on, present, *show_advanced);
        }

        section_toggles(ui, show_advanced, show_channels, !channels.is_empty());

        // Basic groups, each separated by a divider.  Adding a new metric family
        // is just another `chip_group` call with its `MetricKind` slice.
        let basic_groups: [&[MetricKind]; 2] = [
            // Summary metrics (total satellite counts, velocity, EPH, heading, clock delta).
            &[
                MetricKind::SatsSeen,
                MetricKind::SatsFix,
                MetricKind::Velocity,
                MetricKind::Eph,
                MetricKind::SnapError,
                MetricKind::HeadingDeg,
                MetricKind::ClockDeltaMs,
            ],
            // Per-constellation satellite counts.  Chips for a constellation
            // absent from the loaded data are skipped by `chip_group`.
            &[
                MetricKind::GpsSeen,
                MetricKind::GpsFix,
                MetricKind::GlonassSeen,
                MetricKind::GlonassFix,
                MetricKind::GalileoSeen,
                MetricKind::GalileoFix,
                MetricKind::BeidouSeen,
                MetricKind::BeidouFix,
                MetricKind::NavicSeen,
                MetricKind::NavicFix,
                MetricKind::QzssSeen,
                MetricKind::QzssFix,
            ],
        ];
        for group in basic_groups {
            chip_group(
                ui,
                vis,
                present,
                group,
                *show_advanced,
                snap_error_available,
                &mut show_only,
                &mut hovered_chip,
            );
        }

        // Advanced groups, shown only when revealed.  Every kind here must report
        // `MetricKindUi::is_advanced() == true` so line drawing and the
        // show/hide-all scope stay consistent with these chips' visibility.
        if *show_advanced {
            let advanced_groups: [&[MetricKind]; 2] = [
                // Satellite utilization rate.
                &[
                    MetricKind::UtilAll,
                    MetricKind::UtilGps,
                    MetricKind::UtilGlonass,
                    MetricKind::UtilGalileo,
                    MetricKind::UtilBeidou,
                    MetricKind::UtilNavic,
                    MetricKind::UtilQzss,
                ],
                // Loss-of-lock (slip) rate.
                &[
                    MetricKind::SlipAll,
                    MetricKind::SlipGps,
                    MetricKind::SlipGlonass,
                    MetricKind::SlipGalileo,
                    MetricKind::SlipBeidou,
                    MetricKind::SlipNavic,
                    MetricKind::SlipQzss,
                ],
            ];
            for group in advanced_groups {
                chip_group(
                    ui,
                    vis,
                    present,
                    group,
                    *show_advanced,
                    snap_error_available,
                    &mut show_only,
                    &mut hovered_chip,
                );
            }
        }

        // Channel chips, shown only when revealed. One chip per channel: a
        // vector channel toggles all its component lines together.
        if *show_channels && !channels.is_empty() {
            channel_chip_group(
                ui,
                channels,
                channel_vis,
                &mut show_only_channel,
                &mut hovered_chip,
            );
        }
    });

    // Apply "Show only this" - disable the shown metrics and channels, then
    // re-enable the chosen one.
    if show_only.is_some() || show_only_channel.is_some() {
        vis.set_all(false, present, *show_advanced);
        if *show_channels {
            for channel in channels {
                channel_vis.set(&channel.name, false);
            }
        }
    }
    if let Some(kind) = show_only {
        *vis.field_mut(kind) = true;
    }
    if let Some(name) = show_only_channel {
        channel_vis.set(&name, true);
    }

    hovered_chip
}

/// Body of the plot display-settings popup: line width and grid visibility.
///
/// The popup does not capture the plot behind it, so slider edits self-preview
/// live on the lines underneath.
fn plot_display_menu(ui: &mut egui::Ui, show_grid: &mut bool, line_width: &mut f32) {
    ui.set_max_width(220.0);
    ui.strong("Plot display");
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Line width");
        ui.add(
            Slider::new(line_width, PLOT_LINE_WIDTH_RANGE)
                .step_by(0.25)
                .fixed_decimals(2),
        );
    });
    ui.checkbox(show_grid, "Show grid");
    ui.separator();
    let reset_label = format!("{ICON_ARROW_COUNTER_CLOCKWISE} Restore defaults");
    if ui.button(reset_label).clicked() {
        *line_width = DEFAULT_PLOT_LINE_WIDTH;
        *show_grid = true;
    }
}

/// A [`metric_chip`] rendered disabled: off-state visuals, no interaction,
/// hover text explaining what to do first.
fn disabled_metric_chip(ui: &mut egui::Ui, name: &str, color: Color32, hover: &str) {
    let btn = Button::new(RichText::new(name).color(Color32::from_gray(100)).small())
        .fill(color.gamma_multiply(0.12))
        .corner_radius(4.0);
    ui.add_enabled(false, btn).on_disabled_hover_text(hover);
}

/// A small colored toggle chip.  Left-click toggles the metric.  Right-click
/// opens a context menu with "Show only this".
///
/// Returns `(show_only, hovered)` - `show_only` is `true` when the user chose
/// "Show only this" from the context menu.  `hovered` is `true` while the pointer
/// is over this chip.
fn metric_chip(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    name: &str,
    color: Color32,
    tooltip: Option<&str>,
) -> (bool, bool) {
    let (show_only, hovered, _) = chip_button(ui, enabled, name, color, tooltip);
    (show_only, hovered)
}

/// The shared chip widget behind [`metric_chip`] and [`channel_chip`]: the
/// toggle button, its tooltip, and the show-only context menu. Returns the
/// chip's rect so [`channel_chip`] can paint its component bars over it.
fn chip_button(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    name: &str,
    color: Color32,
    tooltip: Option<&str>,
) -> (bool, bool, egui::Rect) {
    let fill = if *enabled {
        color.gamma_multiply(0.75)
    } else {
        color.gamma_multiply(0.12)
    };
    let text_color = if *enabled {
        Color32::WHITE
    } else {
        Color32::from_gray(100)
    };
    let btn = Button::new(RichText::new(name).color(text_color).small())
        .fill(fill)
        .corner_radius(4.0);
    let response = ui.add(btn);
    if response.clicked() {
        *enabled = !*enabled;
    }
    if let Some(tip) = tooltip {
        response.clone().on_hover_text(tip);
    }
    let mut show_only = false;
    response.context_menu(|ui| {
        if ui.button("Show only this").clicked() {
            show_only = true;
            ui.close();
        }
    });
    (show_only, response.hovered(), response.rect)
}

/// Height of the component color bars along a channel chip's bottom edge.
const CHIP_BAR_HEIGHT: f32 = 3.0;

/// Gap between adjacent component color bars, in points.
const CHIP_BAR_GAP: f32 = 1.0;

/// Corner radius of one component bar - subtler than the chip's 4.0, a bar
/// is only [`CHIP_BAR_HEIGHT`] tall.
const CHIP_BAR_CORNER_RADIUS: f32 = 1.0;

/// Alpha of the component bars on a disabled chip. Stronger than the chip
/// fill's 0.12: the bars are a few pixels tall and vanish entirely at the
/// fill's dimming, and they are the only place the component colors show.
const CHIP_BAR_DISABLED_ALPHA: f32 = 0.25;

/// A channel's chip: the metric chip extended with a bar strip along the
/// bottom edge, one bar per component in that component's line color - the
/// legend for a vector channel's x/y/z hues.
fn channel_chip(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    name: &str,
    color: Color32,
    components: NonZeroUsize,
    tooltip: Option<&str>,
) -> (bool, bool) {
    let (show_only, hovered, rect) = chip_button(ui, enabled, name, color, tooltip);
    let components = components.get();
    let strip = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - CHIP_BAR_HEIGHT),
        rect.max,
    );
    let bar_width =
        (strip.width() - CHIP_BAR_GAP * (components.saturating_sub(1)) as f32) / components as f32;
    // The bars dim with the chip, so a disabled channel stays quiet.
    let alpha = if *enabled {
        1.0
    } else {
        CHIP_BAR_DISABLED_ALPHA
    };
    for index in 0..components {
        let left = strip.left() + index as f32 * (bar_width + CHIP_BAR_GAP);
        let bar = egui::Rect::from_min_size(
            egui::pos2(left, strip.top()),
            egui::vec2(bar_width, strip.height()),
        );
        ui.painter().rect_filled(
            bar,
            CHIP_BAR_CORNER_RADIUS,
            component_color(color, index).gamma_multiply(alpha),
        );
    }
    (show_only, hovered)
}

/// Compute fresh level selections for all metrics of one track's series.
///
/// The sample target is derived per track from how many pixels the track
/// occupies in the current view ([`track_target`]), so a track that is only a
/// few pixels wide selects a coarse mipmap level with just a few points.
fn compute_level_cache(
    series: &TrackSeries,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    sample_cap: usize,
) -> TripLevelCache {
    let target = track_target(series.x_range, x_min, x_max, available_width, sample_cap);
    let sel = |mm: &MipMap| mm.select_indices(x_min, x_max, target);
    TripLevelCache {
        total_seen: sel(&series.total_seen),
        total_fix: sel(&series.total_fix),
        gps_seen: sel(&series.gps_seen),
        gps_fix: sel(&series.gps_fix),
        glonass_seen: sel(&series.glonass_seen),
        glonass_fix: sel(&series.glonass_fix),
        galileo_seen: sel(&series.galileo_seen),
        galileo_fix: sel(&series.galileo_fix),
        beidou_seen: sel(&series.beidou_seen),
        beidou_fix: sel(&series.beidou_fix),
        navic_seen: sel(&series.navic_seen),
        navic_fix: sel(&series.navic_fix),
        qzss_seen: sel(&series.qzss_seen),
        qzss_fix: sel(&series.qzss_fix),
        velocity_kmh: sel(&series.velocity_kmh),
        eph_m: sel(&series.eph_m),
        heading_deg: sel(&series.heading_deg),
        clock_delta_ms: sel(&series.clock_delta_ms),
        util_all: sel(&series.util_all),
        util_gps: sel(&series.util_gps),
        util_glonass: sel(&series.util_glonass),
        util_galileo: sel(&series.util_galileo),
        util_beidou: sel(&series.util_beidou),
        util_navic: sel(&series.util_navic),
        util_qzss: sel(&series.util_qzss),
        slip_all: sel(&series.slip_all),
        slip_gps: sel(&series.slip_gps),
        slip_glonass: sel(&series.slip_glonass),
        slip_galileo: sel(&series.slip_galileo),
        slip_beidou: sel(&series.slip_beidou),
        slip_navic: sel(&series.slip_navic),
        slip_qzss: sel(&series.slip_qzss),
        channels: series
            .channels
            .iter()
            .map(|c| c.components.iter().map(|comp| sel(&comp.mipmap)).collect())
            .collect(),
    }
}

/// Returns `true` when the track at `(fi, ti)` passes visibility and filter checks.
fn trip_is_visible(
    visibility: &TrackDataVisibility,
    global_filter: &GlobalFilter,
    files: &[LoadedFile],
    fi: usize,
    ti: usize,
) -> bool {
    let Some(file_vis) = visibility.files.get(fi) else {
        return false;
    };
    if !file_vis.enabled {
        return false;
    }
    let Some(trip_vis) = file_vis.tracks.get(ti) else {
        return false;
    };
    if !trip_vis.enabled {
        return false;
    }
    let Some(file) = files.get(fi) else {
        return false;
    };
    let Some(track) = file.tracks.get(ti) else {
        return false;
    };
    gt_filter::track_passes_filter(&track.metadata, global_filter)
}

/// Add all metric lines for one track to the plot using pre-computed level selections.
///
/// When `hovered_chip` is `Some(kind)`, that metric is highlighted (double stroke
/// width) and every other line is dimmed to 20 % brightness - mirroring the
/// standard egui-plot legend hover behaviour.
///
/// The `'a` lifetime ties both `plot_ui` and `series` together so that
/// [`egui_plot::PlotPoints::Borrowed`] can reference mipmap slices directly
/// without any per-frame allocation.
#[expect(
    clippy::too_many_arguments,
    reason = "per-line rendering needs the plot, series, level cache, visibility/hover state, track focus, and the advanced-section gate"
)]
fn add_series_lines<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    series: &'a TrackSeries,
    multi_track: bool,
    cache: &TripLevelCache,
    metric_vis: &MetricVisibility,
    channel_vis: &ChannelVisibility,
    present: &HashSet<Constellation>,
    channels: &[LoadedChannel],
    hovered_chip: Option<&HoveredChip>,
    hover_scope: Option<HighlightScope>,
    sections: SectionGates,
    line_width: f32,
    dark_mode: bool,
    snap_error: Option<&'a SnapErrorPlotCache>,
    snap_viewport: SnapErrorViewport,
    snap_pointer: Option<egui::Pos2>,
    hovered_snap: &mut Option<(f32, SnapErrorHover)>,
) {
    let prefix = if multi_track {
        format!("{}: ", series.label)
    } else {
        String::new()
    };
    let focused = series_matches_hover_scope(series, hover_scope);
    let has_track_focus = hover_scope.is_some();

    // The hover-dim treatment every line shares: full color plus highlight
    // while its own chip is hovered, dimmed while any other chip is.
    let hover_treatment = |base: Color32, is_hovered_chip: bool| {
        let (mut color, highlighted) = match hovered_chip {
            Some(_) if is_hovered_chip => (base, true),
            Some(_) => (base.gamma_multiply(0.2), false),
            None => (base, false),
        };
        if has_track_focus && !focused {
            color = color.gamma_multiply(0.2);
        }
        (color, highlighted || (has_track_focus && focused))
    };
    let line_style = file_line_style(series.fi);

    for kind in MetricKind::iter() {
        // Skip metrics with no chip on screen - collapsed advanced section, or a
        // per-constellation metric whose constellation is absent from the data -
        // so a hidden chip never leaves a stray line on the plot.
        if !metric_is_shown(kind, present, sections.show_advanced) {
            continue;
        }
        if !metric_vis.field(kind) {
            continue;
        }
        let is_hovered = hovered_chip == Some(&HoveredChip::Metric(kind));
        let (color, highlighted) =
            hover_treatment(metric_line_color(kind, series.fi, dark_mode), is_hovered);
        // Snap error has no mipmap; it draws from the external per-run
        // series right after this loop.
        let (Some(mipmap), Some(level)) = (series.mipmap_for(kind), cache.level_for(kind)) else {
            continue;
        };
        add_line(
            plot_ui,
            mipmap.slice_at(level),
            format!("{prefix}{}", kind.label()),
            color,
            line_style,
            line_width,
            highlighted,
        );
    }

    if metric_vis.field(MetricKind::SnapError)
        && let Some(snap_cache) = snap_error
    {
        let is_hovered = hovered_chip == Some(&HoveredChip::Metric(MetricKind::SnapError));
        let (color, highlighted) = hover_treatment(
            metric_line_color(MetricKind::SnapError, series.fi, dark_mode),
            is_hovered,
        );
        add_snap_error_series(
            plot_ui,
            &prefix,
            series,
            multi_track,
            snap_cache,
            snap_viewport,
            snap_pointer,
            hovered_snap,
            SnapErrorStyle {
                color,
                style: line_style,
                width: line_width,
                highlighted,
                dark_mode,
            },
        );
    }

    // Channel lines, one per component, gated like the chips: the whole
    // section while collapsed, then the per-channel toggle.
    if !sections.show_channels {
        return;
    }
    for (channel, selections) in series.channels.iter().zip(&cache.channels) {
        if !channel_vis.is_visible(&channel.name) {
            continue;
        }
        // The palette index comes from the cross-file union, so `accel` keeps
        // one hue no matter which file's track is drawing.
        let Some(color_index) = channels
            .iter()
            .find(|c| c.name == channel.name)
            .map(|c| c.color_index)
        else {
            continue;
        };
        let is_hovered =
            matches!(hovered_chip, Some(HoveredChip::Channel(name)) if *name == channel.name);
        let base = channel_line_color(color_index, series.fi);
        let unit_suffix = channel
            .unit
            .as_deref()
            .map_or(String::new(), |u| format!(" ({u})"));
        for (index, (component, selection)) in channel.components.iter().zip(selections).enumerate()
        {
            // Rotate before the hover treatment, so dimming applies to the
            // component's own hue.
            let (color, highlighted) = hover_treatment(component_color(base, index), is_hovered);
            add_line(
                plot_ui,
                component.mipmap.slice_at(*selection),
                format!("{prefix}{}{unit_suffix}", component.label),
                color,
                line_style,
                line_width,
                highlighted,
            );
        }
    }
}

/// Surface the custom nearest-point hovers - the masked-out satellites of an
/// anomaly marker, and the kind and error of a snap point - as tooltips
/// anchored at the pointer. These items suppress egui_plot's own labels.
fn show_nearest_point_tooltips(
    ui: &egui::Ui,
    response: &egui::Response,
    hovered_anomaly: Option<(f32, AnomalyHover)>,
    hovered_snap: Option<(f32, SnapErrorHover)>,
) {
    if !response.hovered() {
        return;
    }
    if let Some((_, hover)) = hovered_anomaly {
        pointer_tooltip(ui, response, "util_anomaly_tooltip", |ui| hover.show(ui));
    }
    if let Some((_, hover)) = hovered_snap {
        pointer_tooltip(ui, response, "snap_error_tooltip", |ui| hover.show(ui));
    }
}

/// A custom tooltip anchored at the pointer, for the nearest-point hovers
/// (anomaly markers, snap error points) that suppress egui_plot's own labels.
fn pointer_tooltip(
    ui: &egui::Ui,
    response: &egui::Response,
    id: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    Tooltip::always_open(
        ui.ctx().clone(),
        response.layer_id,
        egui::Id::new(id),
        egui::PopupAnchor::Pointer,
    )
    .gap(ANOMALY_TOOLTIP_GAP)
    .show(add_contents);
}

/// Whether any visible track has an entry in the snap error series.
fn snap_error_available(
    series_cache: &[TrackSeries],
    visible: &[bool],
    snap_error: &SnapErrorSeries,
) -> bool {
    series_cache
        .iter()
        .zip(visible.iter())
        .any(|(series, &is_vis)| {
            is_vis
                && snap_error
                    .points_by_track
                    .contains_key(&series_track_ref(series))
        })
}

/// The [`TrackRef`] a series was built from, for keying into per-track
/// external data like the snap error series.
fn series_track_ref(series: &TrackSeries) -> TrackRef {
    TrackRef::new(FileIdx::new(series.fi), TrackIdx::new(series.ti))
}

fn series_matches_hover_scope(series: &TrackSeries, hover_scope: Option<HighlightScope>) -> bool {
    match hover_scope {
        Some(HighlightScope::File { file_index }) => file_index.as_usize() == series.fi,
        Some(HighlightScope::Track(track)) | Some(HighlightScope::TrackCategory { track, .. }) => {
            track.fi.as_usize() == series.fi && track.index.as_usize() == series.ti
        }
        Some(HighlightScope::Point(_)) | None => true,
    }
}

/// Submit one metric line to the plot, borrowing the point slice directly via
/// [`PlotPoints::Borrowed`] - no allocation.
///
/// Skips slices with fewer than two points: a single-point line produces no
/// visible geometry and would clutter the legend.
///
/// The shared lifetime `'a` ensures the borrowed slice lives at least as long
/// as the `PlotUi` that will consume it (required because `PlotUi<'a>` is
/// invariant over `'a`).
fn add_line<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    data: &'a [egui_plot::PlotPoint],
    name: String,
    color: Color32,
    style: LineStyle,
    width: f32,
    highlighted: bool,
) {
    if data.len() < 2 {
        return;
    }
    plot_ui.line(
        Line::new(name, PlotPoints::Borrowed(data))
            .color(color)
            .style(style)
            .width(width)
            .highlight(highlighted),
    );
}

/// Pre-formatted tooltip contents for one masked-satellite anomaly marker.
struct AnomalyHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    time: String,
    /// One line per masked-out satellite, e.g. `GPS 07 - 12.3°`.
    sats: Vec<String>,
}

impl AnomalyHover {
    fn new(series: &TrackSeries, multi_track: bool, anomaly: &UtilAnomaly) -> Self {
        let time = DateTime::from_timestamp(anomaly.t as i64, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        let sats = anomaly
            .masked
            .iter()
            .map(|m| {
                format!(
                    "{} {:02} - {:.1}°",
                    m.constellation.display_name(),
                    m.prn,
                    m.elevation
                )
            })
            .collect();
        Self {
            track: multi_track.then(|| series.label.clone()),
            time,
            sats,
        }
    }

    fn show(&self, ui: &mut egui::Ui) {
        ui.strong("Used satellites below the elevation mask");
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.label(format!("at {}", self.time));
        ui.separator();
        for line in &self.sats {
            ui.label(line);
        }
    }
}

/// Draw the masked-satellite anomaly markers for one track and, when the pointer
/// is within [`ANOMALY_HOVER_RADIUS_PX`] of a marker, record the nearest one in
/// `nearest` so the caller can show its tooltip.
///
/// Each marker sits on the all-constellations utilization line at the epoch
/// where the receiver used a satellite below the elevation mask.
fn add_util_anomalies<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    series: &'a TrackSeries,
    multi_track: bool,
    pointer: Option<egui::Pos2>,
    nearest: &mut Option<(f32, AnomalyHover)>,
    dark_mode: bool,
) {
    if series.util_anomalies.is_empty() {
        return;
    }

    let points: Vec<PlotPoint> = series
        .util_anomalies
        .iter()
        .map(|a| PlotPoint::new(a.t, a.value))
        .collect();
    plot_ui.points(
        Points::new("Masked-out used satellites", PlotPoints::Owned(points))
            .shape(MarkerShape::Cross)
            .color(gt_ui_theme::error_indicator(dark_mode))
            .radius(ANOMALY_MARKER_RADIUS)
            // Hover is handled with a custom tooltip, so suppress egui_plot's own.
            .allow_hover(false),
    );

    let Some(ptr) = pointer else {
        return;
    };
    for anomaly in &series.util_anomalies {
        let screen = plot_ui.screen_from_plot(PlotPoint::new(anomaly.t, anomaly.value));
        let dist = screen.distance(ptr);
        if dist <= ANOMALY_HOVER_RADIUS_PX && nearest.as_ref().is_none_or(|(d, _)| dist < *d) {
            *nearest = Some((dist, AnomalyHover::new(series, multi_track, anomaly)));
        }
    }
}

/// Plot y where an unsnapped point's marker sits: rejected points carry no
/// error value, so the marker rests on the axis baseline and the hover text
/// explains the rejection.
const UNSNAPPED_MARKER_Y: f64 = 0.0;

/// Radius of the snapped-point markers on the snap error line. Small - the
/// markers annotate the line's anchor points, they are not anomaly flags.
const SNAPPED_MARKER_RADIUS: f32 = 2.5;

/// Per-track plot-side cache of a snap error series: the line runs as
/// mipmap cascades (downsampled like every other metric), plus the raw
/// per-kind point lists for the marker overlays. Rebuilt only when the
/// track's series [`Arc`] changes - the app hands out one `Arc` per
/// completed run, so this rebuilds once per run, not per frame.
#[derive(Debug, Clone)]
pub(crate) struct SnapErrorPlotCache {
    /// Identity of the source series, for invalidation.
    source: ArcIdentity,
    /// One cascade per drawable line run (maximal valued stretches; see
    /// [`snap_error_runs`]).
    runs: Vec<MipMap>,
    /// Snapped-kind points, ascending by x - the anchor markers.
    snapped: Vec<PlotPoint>,
    /// Unsnapped points at the baseline, ascending by x.
    unsnapped: Vec<PlotPoint>,
}

/// Bring the per-track snap caches in line with the frame's series: drop
/// tracks that left the series, (re)build entries whose source changed.
fn sync_snap_error_cache(
    cache: &mut HashMap<TrackRef, SnapErrorPlotCache>,
    series: &SnapErrorSeries,
) {
    cache.retain(|track, _| series.points_by_track.contains_key(track));
    for (&track, points) in &series.points_by_track {
        let source = ArcIdentity::of(points);
        if cache.get(&track).is_some_and(|c| c.source == source) {
            continue;
        }
        let runs = snap_error_runs(points)
            .into_iter()
            .map(|run| MipMap::build(run.iter().map(|p| [p.x, p.y]).collect()))
            .collect();
        let snapped = points
            .iter()
            .filter(|p| p.kind == SnapErrorKind::Snapped)
            .filter_map(|p| p.error_m.map(|e| PlotPoint::new(p.x_secs, e)))
            .collect();
        let unsnapped = points
            .iter()
            .filter(|p| p.kind == SnapErrorKind::Unsnapped)
            .map(|p| PlotPoint::new(p.x_secs, UNSNAPPED_MARKER_Y))
            .collect();
        cache.insert(
            track,
            SnapErrorPlotCache {
                source,
                runs,
                snapped,
                unsnapped,
            },
        );
    }
}

/// The viewport parameters the snap error series selects its mipmap levels
/// with - the same inputs every other metric's level selection uses.
#[derive(Debug, Clone, Copy)]
struct SnapErrorViewport {
    x_min: f64,
    x_max: f64,
    width: f32,
    cap: usize,
}

/// Stroke/hover treatment for one track's snap error series, bundled so
/// [`add_snap_error_series`] stays under the argument-count lint.
#[derive(Clone, Copy)]
struct SnapErrorStyle {
    color: Color32,
    style: LineStyle,
    width: f32,
    highlighted: bool,
    dark_mode: bool,
}

/// Pre-formatted tooltip contents for one hovered unsnapped marker - the
/// only snap point with a custom hover: the kind is the whole message, and
/// the marker sits at the baseline rather than on the line. Snapped and
/// interpolated points hover natively through egui_plot's labels.
struct SnapErrorHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    time: String,
}

impl SnapErrorHover {
    fn new(series: &TrackSeries, multi_track: bool, x_secs: f64) -> Self {
        let time = DateTime::from_timestamp(x_secs as i64, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        Self {
            track: multi_track.then(|| series.label.clone()),
            time,
        }
    }

    fn show(&self, ui: &mut egui::Ui) {
        ui.strong("Unsnapped");
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.label(&self.time);
        ui.label("The road network rejected this point");
    }
}

/// The drawable line runs of a snap error series: maximal stretches of
/// consecutive valued points, split wherever a point carries no error (the
/// road network rejected it, so the line honestly breaks there). Runs of a
/// single point produce no visible line geometry and are dropped - the
/// point's value stays reachable through the custom hover.
fn snap_error_runs(points: &[SnapErrorPoint]) -> Vec<Vec<PlotPoint>> {
    let mut runs = Vec::new();
    let mut run: Vec<PlotPoint> = Vec::new();
    let mut flush = |run: &mut Vec<PlotPoint>| {
        if run.len() >= 2 {
            runs.push(std::mem::take(run));
        } else {
            run.clear();
        }
    };
    for point in points {
        match point.error_m {
            Some(error) => run.push(PlotPoint::new(point.x_secs, error)),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    runs
}

/// Draw one track's snap error series from its plot cache: mipmapped line
/// runs split at unsnapped points (the road network rejected those, so the
/// line honestly breaks), snapped-point anchor markers while zoomed to full
/// detail, and a baseline cross per unsnapped point.
///
/// The line and the anchor markers hover natively - egui_plot places the
/// standard name/time/value label on the line, interpolated between points,
/// so nothing clips at the plot edge. Only the unsnapped crosses keep a
/// custom tooltip: there the kind is the whole message.
/// Level selection per run, exactly like the other metrics, plus whether
/// every viewport-visible run reads its finest level. The anchor markers
/// only draw at full detail - coarser levels merge points, so a marker
/// would no longer name a real point. Runs outside the viewport neither
/// draw nor veto the markers.
fn select_run_levels(runs: &[MipMap], viewport: SnapErrorViewport) -> (Vec<LevelSelection>, bool) {
    let mut full_detail = true;
    let selections = runs
        .iter()
        .map(|run| {
            let target = track_target(
                run.x_range(),
                viewport.x_min,
                viewport.x_max,
                viewport.width,
                viewport.cap,
            );
            let selection = run.select_indices(viewport.x_min, viewport.x_max, target);
            // Only runs whose data actually intersects the viewport get a
            // vote: a selection always keeps one boundary point (so lines
            // stay connected off-screen), so slice emptiness cannot tell
            // an off-viewport run apart - and its zero visible width
            // forces a coarse level that would wrongly veto the markers.
            let visible = run
                .x_range()
                .is_some_and(|(lo, hi)| lo <= viewport.x_max && hi >= viewport.x_min);
            if visible {
                full_detail &= selection.is_full_detail();
            }
            selection
        })
        .collect();
    (selections, full_detail)
}

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors add_series_lines' argument list; a struct would only relabel it"
)]
fn add_snap_error_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    prefix: &str,
    series: &TrackSeries,
    multi_track: bool,
    cache: &'a SnapErrorPlotCache,
    viewport: SnapErrorViewport,
    pointer: Option<egui::Pos2>,
    nearest: &mut Option<(f32, SnapErrorHover)>,
    style: SnapErrorStyle,
) {
    let (selections, full_detail) = select_run_levels(&cache.runs, viewport);
    for (run, selection) in cache.runs.iter().zip(selections) {
        add_line(
            plot_ui,
            run.slice_at(selection),
            format!("{prefix}{}", MetricKind::SnapError.label()),
            style.color,
            style.style,
            style.width,
            style.highlighted,
        );
    }

    if full_detail && !cache.snapped.is_empty() {
        let start = cache.snapped.partition_point(|p| p.x < viewport.x_min);
        let end = cache.snapped.partition_point(|p| p.x <= viewport.x_max);
        let visible = cache.snapped.get(start..end).unwrap_or_default();
        if !visible.is_empty() {
            plot_ui.points(
                Points::new(
                    format!("{prefix}{} - snapped", MetricKind::SnapError.label()),
                    PlotPoints::Borrowed(visible),
                )
                .shape(MarkerShape::Circle)
                .color(style.color)
                .radius(SNAPPED_MARKER_RADIUS)
                .highlight(style.highlighted),
            );
        }
    }

    if !cache.unsnapped.is_empty() {
        plot_ui.points(
            Points::new("Unsnapped points", PlotPoints::Borrowed(&cache.unsnapped))
                .shape(MarkerShape::Cross)
                .color(gt_ui_theme::error_indicator(style.dark_mode))
                .radius(ANOMALY_MARKER_RADIUS)
                .allow_hover(false),
        );
    }

    let Some(ptr) = pointer else {
        return;
    };
    for point in &cache.unsnapped {
        let screen = plot_ui.screen_from_plot(*point);
        let dist = screen.distance(ptr);
        if dist <= ANOMALY_HOVER_RADIUS_PX && nearest.as_ref().is_none_or(|(d, _)| dist < *d) {
            *nearest = Some((dist, SnapErrorHover::new(series, multi_track, point.x)));
        }
    }
}

/// Given a set of visible tracks and a plot-hovered time (in seconds since
/// epoch), find the `(file_index, track_index, point_index)` of the closest
/// TPV point across all visible tracks.
///
/// Called by the app to set `MapHighlight::plot_hover_time` after the plot renders.
pub fn find_closest_tpv(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    target: DateTime<Utc>,
) -> Option<(FileIdx, TrackIdx, PointIdx)> {
    let target_secs = target.timestamp() as f64;
    let mut best: Option<(FileIdx, TrackIdx, PointIdx, f64)> = None;

    for (fi, file) in files.iter().enumerate() {
        let fi = FileIdx::new(fi);
        let Some(file_vis) = fi.get(&visibility.files) else {
            continue;
        };
        if !file_vis.enabled {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            let ti = TrackIdx::new(ti);
            let Some(trip_vis) = ti.get(&file_vis.tracks) else {
                continue;
            };
            if !trip_vis.enabled {
                continue;
            }
            if !gt_filter::track_passes_filter(&track.metadata, filter) {
                continue;
            }
            // Only points inside the filter's time window are drawn on the map,
            // so the cross-highlight must pick the nearest *visible* point - never
            // one filtered out of view.
            let nearest = track
                .points
                .iter()
                .enumerate()
                .filter(|(_, p)| gt_filter::point_passes_time_filter(p.tpv.time().utc(), filter))
                .map(|(i, p)| (i, (p.tpv.time().as_secs_f64() - target_secs).abs()))
                .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let Some((pi, dist)) = nearest else {
                continue;
            };
            if best.is_none_or(|(_, _, _, d)| dist < d) {
                best = Some((fi, ti, PointIdx::new(pi), dist));
            }
        }
    }

    best.map(|(fi, ti, pi, _)| (fi, ti, pi))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A viewport over `0..=x_max` seconds, `width` px wide, with the
    /// given per-track sample cap.
    fn viewport(x_max: f64, width: f32, cap: usize) -> SnapErrorViewport {
        SnapErrorViewport {
            x_min: 0.0,
            x_max,
            width,
            cap,
        }
    }

    /// A run cascade over `count` one-per-second points starting at
    /// `start` seconds.
    fn run_from(start: usize, count: usize) -> MipMap {
        MipMap::build((0..count).map(|i| [(start + i) as f64, 1.0]).collect())
    }

    /// A run cascade over `count` one-per-second points from time zero.
    fn run_of(count: usize) -> MipMap {
        run_from(0, count)
    }

    /// The anchor-marker gate: dots draw only while every viewport-visible
    /// run reads its finest mipmap level. A downsampled run vetoes; a run
    /// entirely outside the viewport neither draws nor vetoes; no runs at
    /// all leave the gate open (the marker overlay then has nothing to
    /// draw from anyway, but the gate must not mask a future source).
    #[rstest::rstest]
    #[case::no_runs(vec![], viewport(100.0, 800.0, 4096), true)]
    #[case::single_run_at_full_detail(vec![run_of(100)], viewport(100.0, 800.0, 4096), true)]
    #[case::single_run_downsampled(vec![run_of(4096)], viewport(4096.0, 100.0, 64), false)]
    #[case::downsampled_run_vetoes_the_full_one(
        vec![run_of(100), run_of(4096)],
        viewport(4096.0, 100.0, 64),
        false
    )]
    #[case::off_viewport_run_does_not_veto(
        vec![run_of(64), run_from(100_000, 4096)],
        viewport(100.0, 800.0, 4096),
        true
    )]
    fn marker_gate_requires_full_detail_on_every_visible_run(
        #[case] runs: Vec<MipMap>,
        #[case] viewport: SnapErrorViewport,
        #[case] expected: bool,
    ) {
        let (selections, full_detail) = select_run_levels(&runs, viewport);
        assert_eq!(selections.len(), runs.len(), "one selection per run");
        assert_eq!(full_detail, expected);
    }

    /// The component hue ladder: the first component keeps the channel
    /// color, later ones alternate around it in fixed hue steps and wrap
    /// cleanly around the hue circle - always distinct from the base.
    #[rstest::rstest]
    #[case::base(0, 0.0)]
    #[case::second_steps_up(1, 60.0 / 360.0)]
    #[case::third_steps_down(2, -60.0 / 360.0)]
    #[case::fourth_steps_further(3, 120.0 / 360.0)]
    #[case::fifth_steps_further_down(4, -120.0 / 360.0)]
    fn component_colors_ladder_around_the_base_hue(#[case] component: usize, #[case] offset: f32) {
        let base = CHANNEL_PALETTE[0];
        let base_hue = egui::ecolor::Hsva::from(base).h;
        let got = egui::ecolor::Hsva::from(component_color(base, component)).h;
        let want = (base_hue + offset).rem_euclid(1.0);
        assert!(
            (got - want).abs() < 0.01 || (got - want).abs() > 0.99,
            "component {component}: hue {got} != {want}"
        );
    }

    /// The plot-side snap cache follows the series by `Arc` identity: an
    /// unchanged `Arc` is reused, a replaced one rebuilds its entry, and a
    /// track that left the series is pruned.
    #[test]
    fn snap_cache_syncs_by_arc_identity() {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let points = Arc::new(vec![
            sep(0.0, Some(1.0)),
            sep(1.0, Some(2.0)),
            sep(2.0, None),
            sep(3.0, Some(3.0)),
            sep(4.0, Some(4.0)),
        ]);
        let mut series = SnapErrorSeries::default();
        series.points_by_track.insert(track, Arc::clone(&points));

        let mut cache = HashMap::new();
        sync_snap_error_cache(&mut cache, &series);
        let entry = cache.get(&track).expect("entry built");
        assert_eq!(entry.runs.len(), 2, "one cascade per line run");
        assert_eq!(entry.snapped.len(), 4);
        assert_eq!(entry.unsnapped.len(), 1);
        let built_source = entry.source;

        // Same Arc: the entry is reused, not rebuilt.
        sync_snap_error_cache(&mut cache, &series);
        assert_eq!(
            cache.get(&track).map(|e| e.source),
            Some(built_source),
            "an unchanged series keeps its cache entry"
        );

        // A new run (new Arc) rebuilds; a removed track prunes.
        series.points_by_track.insert(
            track,
            Arc::new(vec![sep(0.0, Some(1.0)), sep(1.0, Some(2.0))]),
        );
        sync_snap_error_cache(&mut cache, &series);
        assert_ne!(cache.get(&track).map(|e| e.source), Some(built_source));
        assert_eq!(cache.get(&track).map(|e| e.runs.len()), Some(1));

        series.points_by_track.clear();
        sync_snap_error_cache(&mut cache, &series);
        assert!(cache.is_empty(), "tracks that left the series are pruned");
    }

    /// Shorthand for a snap error point at time `x` with the given error.
    fn sep(x: f64, error_m: Option<f64>) -> SnapErrorPoint {
        SnapErrorPoint {
            x_secs: x,
            error_m,
            kind: if error_m.is_some() {
                SnapErrorKind::Snapped
            } else {
                SnapErrorKind::Unsnapped
            },
        }
    }

    /// The line runs split exactly at valueless points, and runs of a single
    /// point (leading, interior, or trailing) are dropped - one point draws
    /// no line and would clutter the legend.
    #[rstest::rstest]
    #[case::empty(&[], &[])]
    #[case::one_unbroken_run(&[(0.0, Some(1.0)), (1.0, Some(2.0)), (2.0, Some(3.0))], &[3])]
    #[case::interior_break(
        &[(0.0, Some(1.0)), (1.0, Some(2.0)), (2.0, None), (3.0, Some(4.0)), (4.0, Some(5.0))],
        &[2, 2]
    )]
    #[case::leading_single_point_dropped(
        &[(0.0, Some(1.0)), (1.0, None), (2.0, Some(3.0)), (3.0, Some(4.0))],
        &[2]
    )]
    #[case::trailing_single_point_dropped(
        &[(0.0, Some(1.0)), (1.0, Some(2.0)), (2.0, None), (3.0, Some(4.0))],
        &[2]
    )]
    #[case::all_unsnapped(&[(0.0, None), (1.0, None)], &[])]
    fn snap_error_runs_split_at_valueless_points(
        #[case] input: &[(f64, Option<f64>)],
        #[case] expected_run_lengths: &[usize],
    ) {
        let points: Vec<SnapErrorPoint> = input.iter().map(|&(x, e)| sep(x, e)).collect();
        let runs = snap_error_runs(&points);
        let lengths: Vec<usize> = runs.iter().map(Vec::len).collect();
        assert_eq!(lengths, expected_run_lengths);
        // Every emitted vertex carries its point's own x and error value.
        for run in &runs {
            for vertex in run {
                // The test x values are small exact-in-f64 literals, so
                // bit-equality is the right lookup here.
                let source = points
                    .iter()
                    .find(|p| p.x_secs.to_bits() == vertex.x.to_bits())
                    .expect("vertex maps to a source point");
                assert_eq!(source.error_m, Some(vertex.y));
            }
        }
    }

    /// Every per-constellation metric maps to a constellation and every
    /// all-constellation metric maps to `None`, with the two groups together
    /// covering all `MetricKind::COUNT` variants - so a new metric must declare
    /// which bucket it falls in rather than silently defaulting.
    #[test]
    fn metric_constellation_mapping_is_total() {
        use strum::EnumCount;
        let with = MetricKind::iter()
            .filter(|k| k.constellation().is_some())
            .count();
        let without = MetricKind::iter()
            .filter(|k| k.constellation().is_none())
            .count();
        assert_eq!(with + without, MetricKind::COUNT);
        // 6 constellations x {seen, fix, util, slip}.
        assert_eq!(with, 24);
    }

    /// A per-constellation chip/line shows only when its constellation appears
    /// in the data; all-constellation metrics always show (subject to the
    /// advanced gate).  This is the rule that hides empty NavIC/QZSS chips.
    #[test]
    fn metric_is_shown_gates_on_presence_and_advanced() {
        let none: HashSet<Constellation> = HashSet::new();
        let gps_only: HashSet<Constellation> = std::iter::once(Constellation::Gps).collect();

        // Totals always show regardless of which constellations are present.
        assert!(metric_is_shown(MetricKind::SatsSeen, &none, false));
        // GPS chip hidden with no data, shown once GPS is present.
        assert!(!metric_is_shown(MetricKind::GpsSeen, &none, false));
        assert!(metric_is_shown(MetricKind::GpsSeen, &gps_only, false));
        // NavIC/QZSS stay hidden in a GPS-only recording.
        assert!(!metric_is_shown(MetricKind::NavicSeen, &gps_only, false));
        assert!(!metric_is_shown(MetricKind::QzssFix, &gps_only, false));
        // Advanced metrics need the advanced section open *and* presence.
        assert!(!metric_is_shown(MetricKind::UtilGps, &gps_only, false));
        assert!(metric_is_shown(MetricKind::UtilGps, &gps_only, true));
        assert!(!metric_is_shown(MetricKind::UtilNavic, &gps_only, true));
    }

    #[test]
    fn track_target_scales_with_visible_pixels() {
        let width = 1000.0;
        let cap = single_target(width);

        // A track spanning the whole view gets ~2 points per pixel (width is
        // 1000 px, so a full-width track should exceed that).
        let full = track_target(Some((0.0, 100.0)), 0.0, 100.0, width, cap);
        assert!(full > 1000, "full-width track should be ~2 pts/pixel");
        assert!(full <= cap);

        // A track occupying ~1% of the view (~10 px) hands over only a handful
        // of points - the old fixed target pulled hundreds regardless.
        let tiny = track_target(Some((0.0, 1.0)), 0.0, 100.0, width, cap);
        assert!(tiny >= 2);
        assert!(
            tiny <= 32,
            "few-pixel track must hand over few points, got {tiny}"
        );

        // An empty track stays minimal.  A degenerate (zero-width) view never
        // divides by zero and falls back to the cap.
        assert_eq!(track_target(None, 0.0, 100.0, width, cap), 2);
        assert_eq!(track_target(Some((0.0, 1.0)), 5.0, 5.0, width, cap), cap);
    }

    #[test]
    fn budget_cap_bounds_overlapping_tracks() {
        let width = 1000.0;
        let single = single_target(width);
        let budget = single * BUDGET_TRACK_MULTIPLE;

        // Up to BUDGET_TRACK_MULTIPLE overlapping tracks keep full resolution.
        assert_eq!(budget_cap(width, 1), single);
        assert_eq!(budget_cap(width, BUDGET_TRACK_MULTIPLE), single);

        // Beyond that the cap shares the budget so total full-width points stay
        // bounded (allowing the integer-division remainder).
        for count in [BUDGET_TRACK_MULTIPLE + 1, 50, 500] {
            let cap = budget_cap(width, count);
            assert!((2..=single).contains(&cap));
            assert!(
                cap * count <= budget + count,
                "cap {cap} × {count} exceeds budget {budget}"
            );
        }

        // Zero visible count must not divide by zero.
        assert_eq!(budget_cap(width, 0), single);
    }

    #[test]
    fn file_shading_distinguishes_adjacent_files() {
        let a = metric_line_color(MetricKind::SatsSeen, 0, true);
        let b = metric_line_color(MetricKind::SatsSeen, 1, true);
        assert_ne!(a, b);
    }

    #[test]
    fn sats_seen_and_sats_fix_stay_visually_separate_across_files() {
        // The seen line stays the lighter-green blue and fix the deeper one in
        // both themes, so the pair never collapses into one colour.
        for dark_mode in [true, false] {
            for fi in 0..FILE_SHADE_FACTORS.len() * 5 {
                let seen = metric_line_color(MetricKind::SatsSeen, fi, dark_mode);
                let fix = metric_line_color(MetricKind::SatsFix, fi, dark_mode);
                assert!(
                    seen.g() > fix.g(),
                    "seen should stay the lighter blue: dark={dark_mode}, fi={fi}, seen={seen:?}, fix={fix:?}"
                );
            }
        }
    }

    #[test]
    fn file_line_styles_are_pairwise_distinct() {
        for (i, a) in FILE_LINE_STYLES.iter().enumerate() {
            for (j, b) in FILE_LINE_STYLES.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    a, b,
                    "FILE_LINE_STYLES[{i}] duplicates FILE_LINE_STYLES[{j}]"
                );
            }
        }
    }

    fn test_plot_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))
    }

    #[test]
    fn resolve_legend_offset_clamps_to_plot_edges() {
        let legend_size = egui::vec2(100.0, 50.0);
        let offset = resolve_legend_offset(
            egui::vec2(-50.0, 1000.0),
            legend_size,
            test_plot_rect(),
            false,
            false,
        );
        assert!((offset.x - LEGEND_EDGE_MARGIN).abs() < f32::EPSILON);
        assert!((offset.y - (300.0 - legend_size.y - LEGEND_EDGE_MARGIN)).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_legend_offset_redocks_on_explicit_request() {
        let offset = resolve_legend_offset(
            egui::vec2(200.0, 150.0),
            egui::vec2(100.0, 50.0),
            test_plot_rect(),
            true,
            false,
        );
        assert_eq!(offset, LEGEND_DOCK_OFFSET);
    }

    #[test]
    fn resolve_legend_offset_snaps_to_dock_on_release_near_corner_only() {
        let legend_size = egui::vec2(100.0, 50.0);
        let near_dock = LEGEND_DOCK_OFFSET + egui::vec2(LEGEND_DOCK_SNAP_RADIUS - 1.0, 0.0);

        let mid_drag =
            resolve_legend_offset(near_dock, legend_size, test_plot_rect(), false, false);
        assert_eq!(mid_drag, near_dock, "must not snap before drag release");

        let released = resolve_legend_offset(near_dock, legend_size, test_plot_rect(), false, true);
        assert_eq!(
            released, LEGEND_DOCK_OFFSET,
            "must snap once released near the dock"
        );
    }

    #[test]
    fn resolve_legend_offset_does_not_snap_when_far_from_dock() {
        let legend_size = egui::vec2(100.0, 50.0);
        let far = egui::vec2(200.0, 150.0);
        let offset = resolve_legend_offset(far, legend_size, test_plot_rect(), false, true);
        assert_eq!(offset, far);
    }

    #[test]
    fn channel_visibility_defaults_to_visible_and_remembers_toggles() {
        let mut vis = ChannelVisibility::default();
        assert!(vis.is_visible("accel"), "an untoggled channel is visible");
        vis.set("accel", false);
        assert!(!vis.is_visible("accel"));
        assert!(vis.is_visible("incline"), "other names stay visible");
        vis.set("accel", true);
        assert!(vis.is_visible("accel"));
        vis.set("incline", false);
        assert_eq!(
            vis.entries(),
            vec![("accel".to_owned(), true), ("incline".to_owned(), false)],
            "entries list every toggled name, sorted"
        );
    }

    #[test]
    fn loaded_channels_union_is_sorted_and_deduplicated() {
        use crate::series::{ChannelComponentSeries, ChannelSeries};
        use gt_egui_mipmap::MipMap;

        let channel = |name: &str, unit: Option<&str>, components: usize| ChannelSeries {
            name: name.to_owned(),
            unit: unit.map(str::to_owned),
            components: (0..components)
                .map(|i| ChannelComponentSeries {
                    label: format!("{name}.{i}"),
                    mipmap: MipMap::build(vec![]),
                })
                .collect(),
        };
        // Two tracks' channel lists, flattened like the series cache is:
        // `accel` appears twice and must union to one entry - with the
        // widest component count, so the chip shows every bar even when one
        // file's recording carries fewer components.
        let lists = [
            channel("incline", Some("deg"), 1),
            channel("accel", Some("g"), 1),
            channel("accel", Some("g"), 3),
        ];
        let channels = loaded_channels(lists.iter());
        let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["accel", "incline"], "sorted union across series");
        assert_eq!(channels[0].unit.as_deref(), Some("g"));
        assert_eq!(
            channels[0].components.get(),
            3,
            "the widest series' component count wins"
        );
        // Palette indices follow the sorted order, so a channel keeps one hue
        // across files.
        assert_eq!(channels[0].color_index, 0);
        assert_eq!(channels[1].color_index, 1);
        assert_eq!(channel_color(0), channel_color(CHANNEL_PALETTE.len()));
    }
}
