//! The filter row above the plot: metric and channel chips, their
//! visibility state, hover metadata, and the channel color pickers.

use std::num::NonZeroUsize;

use egui::{Button, Color32, RichText, Slider};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::GAUGE as ICON_GAUGE;
use egui_phosphor::regular::GEAR as ICON_GEAR;
use egui_phosphor::regular::LINK as ICON_LINK;
use egui_phosphor::regular::WAVE_SINE as ICON_WAVE_SINE;
use gt_solar::GeomagneticIndex;
use gt_types::MetricKind;
use gt_types::satellites::{Constellation, ConstellationSet};
use gt_ui_types::MetricChipHover;
use rustc_hash::FxHashMap;
use strum::IntoEnumIterator;

use super::style::{channel_color, effective_component_color};
use super::{DEFAULT_PLOT_LINE_WIDTH, PLOT_LINE_WIDTH_RANGE};

/// What a chip shows on hover: a paragraph of prose, or the three scannable
/// lines of an environment metric.
pub(super) enum ChipHover {
    Paragraph(&'static str),
    Structured(&'static MetricChipHover),
}

impl ChipHover {
    fn attach(self, response: egui::Response) -> egui::Response {
        match self {
            Self::Paragraph(text) => response.on_hover_text(text),
            Self::Structured(hover) => response.on_hover_ui(|ui| {
                ui.strong(&hover.definition);
                ui.label(&hover.source_cadence_and_scale);
                ui.label(RichText::new(hover.reference_line()).weak());
            }),
        }
    }
}

/// Chip color, label, and optional hover tooltip for each [`MetricKind`].
///
/// `MetricKind` lives in `gt_types` (shared with the persisted settings, see
/// `geotrace::settings::PlotSettings::metric`). These are presentation
/// details specific to this widget, so they live here as an extension trait.
/// The `match` in each method forces a compile error here when a variant is
/// added until every arm is filled in.
pub(super) trait MetricKindUi {
    fn label(self) -> &'static str;
    fn hover(self) -> Option<ChipHover>;
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
            | Self::SnapError
            | Self::Jamming
            | Self::Hp30
            | Self::Kp
            | Self::Tec => None,
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
            Self::Jamming => "Aircraft interference (%)",
            Self::Hp30 => "Hp30 index",
            Self::Kp => "Kp index",
            Self::Tec => "TEC (TECU)",
        }
    }

    fn hover(self) -> Option<ChipHover> {
        match self {
            Self::Jamming => Some(ChipHover::Structured(&gt_jam::text::PLOT_HOVER)),
            Self::Tec => Some(ChipHover::Structured(&gt_ionex::text::PLOT_HOVER)),
            Self::Hp30 => Some(ChipHover::Structured(GeomagneticIndex::Hp30.plot_hover())),
            Self::Kp => Some(ChipHover::Structured(GeomagneticIndex::Kp.plot_hover())),
            Self::Eph => Some(ChipHover::Paragraph(
                "Estimated Horizontal Position error - the GPS receiver's own estimate of how \
                 far the reported position may be from the true position, in metres. \
                 Lower is more accurate.",
            )),
            Self::SnapError => Some(ChipHover::Paragraph(
                "Distance from each recorded point to its road-snapped position, in metres - \
                 the observed deviation from the road network. Plot it next to EPH to compare \
                 the receiver's claimed accuracy with the observed deviation. Values exist only \
                 for points sent in a completed snap run. Zoomed in, a dot marks a point the \
                 matcher placed independently; the plain line between dots is interpolated \
                 along the road; a cross at the baseline is a point the road network rejected.",
            )),
            Self::ClockDeltaMs => Some(ChipHover::Paragraph(
                "GPS clock lead over the host system clock, in milliseconds. \
                 Positive = GPS clock ahead of the system clock; negative = system clock ahead. \
                 Only shown when the receiver reports a system timestamp alongside the GPS fix.",
            )),
            Self::UtilAll => Some(ChipHover::Paragraph(
                "Utilization rate, all constellations: satellites used in the fix divided by \
                 satellites in view, both counted above the elevation mask. A red cross marks \
                 where a used satellite fell below the mask and was excluded. Adjust the mask in \
                 Settings.",
            )),
            Self::UtilGps => Some(ChipHover::Paragraph(
                "GPS utilization rate: GPS satellites used in the fix divided by GPS satellites \
                 in view above the elevation mask.",
            )),
            Self::UtilGlonass => Some(ChipHover::Paragraph(
                "GLONASS utilization rate: GLONASS satellites used in the fix divided by GLONASS \
                 satellites in view above the elevation mask.",
            )),
            Self::UtilGalileo => Some(ChipHover::Paragraph(
                "Galileo utilization rate: Galileo satellites used in the fix divided by Galileo \
                 satellites in view above the elevation mask.",
            )),
            Self::UtilBeidou => Some(ChipHover::Paragraph(
                "BeiDou utilization rate: BeiDou satellites used in the fix divided by BeiDou \
                 satellites in view above the elevation mask.",
            )),
            Self::UtilNavic => Some(ChipHover::Paragraph(
                "NavIC utilization rate: NavIC satellites used in the fix divided by NavIC \
                 satellites in view above the elevation mask.",
            )),
            Self::UtilQzss => Some(ChipHover::Paragraph(
                "QZSS utilization rate: QZSS satellites used in the fix divided by QZSS \
                 satellites in view above the elevation mask.",
            )),
            Self::SlipAll => Some(ChipHover::Paragraph(
                "Loss-of-lock (slip) rate per minute, all constellations: how often the receiver \
                 loses a satellite it should still be tracking. A slip is counted when an \
                 above-mask satellite vanishes, or when its SNR drops sharply between epochs. \
                 Averaged over a trailing window. Tune the mask, SNR-drop threshold, and window \
                 in Settings.",
            )),
            Self::SlipGps => Some(ChipHover::Paragraph(
                "GPS loss-of-lock (slip) rate per minute: GPS satellites lost or sharply faded \
                 above the elevation mask, averaged over the slip window.",
            )),
            Self::SlipGlonass => Some(ChipHover::Paragraph(
                "GLONASS loss-of-lock (slip) rate per minute: GLONASS satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            )),
            Self::SlipGalileo => Some(ChipHover::Paragraph(
                "Galileo loss-of-lock (slip) rate per minute: Galileo satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            )),
            Self::SlipBeidou => Some(ChipHover::Paragraph(
                "BeiDou loss-of-lock (slip) rate per minute: BeiDou satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            )),
            Self::SlipNavic => Some(ChipHover::Paragraph(
                "NavIC loss-of-lock (slip) rate per minute: NavIC satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            )),
            Self::SlipQzss => Some(ChipHover::Paragraph(
                "QZSS loss-of-lock (slip) rate per minute: QZSS satellites lost or sharply \
                 faded above the elevation mask, averaged over the slip window.",
            )),
            _ => None,
        }
    }
}

gt_types::enum_bitset! {
    /// One bit per [`MetricKind`], wrapped by [`MetricVisibility`] which adds
    /// the all-visible default and the shown-metric gating helpers.
    struct MetricKindSet(u64) for MetricKind;
}

/// Global per-metric visibility flags.
///
/// Disabling a metric hides it for **all** tracks at once, making it easy to
/// declutter the plot without touching per-track settings.
#[derive(Debug, Clone, Copy)]
pub struct MetricVisibility(MetricKindSet);

impl Default for MetricVisibility {
    fn default() -> Self {
        Self(
            MetricKind::iter()
                .filter(|kind| kind.visible_by_default())
                .collect(),
        )
    }
}

impl MetricVisibility {
    pub fn field(self, kind: MetricKind) -> bool {
        self.0.contains(kind)
    }

    pub fn set(&mut self, kind: MetricKind, enabled: bool) {
        self.0.set(kind, enabled);
    }

    /// Returns `true` when every *currently shown* metric is enabled.  Advanced
    /// metrics are ignored while the advanced section is collapsed (`show_advanced
    /// == false`), and per-constellation metrics whose constellation is absent
    /// from the loaded data are ignored too (their chips are hidden), so the
    /// show/hide-all button neither reflects nor toggles them.
    fn all_enabled(self, present: ConstellationSet, show_advanced: bool) -> bool {
        MetricKind::iter()
            .filter(|&k| metric_is_shown(k, present, show_advanced))
            .all(|k| self.field(k))
    }

    /// Set every *currently shown* metric to `enabled`, leaving hidden metrics
    /// (collapsed advanced section, or an absent constellation) untouched.
    fn set_all(&mut self, enabled: bool, present: ConstellationSet, show_advanced: bool) {
        for k in MetricKind::iter().filter(|&k| metric_is_shown(k, present, show_advanced)) {
            self.set(k, enabled);
        }
    }
}

/// Whether a metric's chip and line should be shown, given which constellations
/// appear in the loaded data and whether the advanced section is revealed.
///
/// A per-constellation metric is shown only when that constellation is present.
/// An advanced metric is shown only when the advanced section is open.  This is
/// the single gate shared by chip rendering, line drawing, and the show/hide-all
/// logic so they never disagree about what is on screen.
pub(super) fn metric_is_shown(
    kind: MetricKind,
    present: ConstellationSet,
    show_advanced: bool,
) -> bool {
    (show_advanced || !kind.is_advanced())
        && kind.constellation().is_none_or(|c| present.contains(c))
}

/// Global per-channel visibility, keyed by channel name.
///
/// A name that was never toggled is visible, matching the persisted-settings
/// convention (missing key = shown).
/// Names persist across loads: an `accel` hidden once stays hidden in the next
/// recording that carries an `accel`.
#[derive(Debug, Clone, Default)]
pub struct ChannelVisibility(FxHashMap<String, bool>);

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
pub(super) struct LoadedChannel {
    pub(super) name: String,
    pub(super) unit: Option<String>,
    pub(super) color_index: usize,
    /// Component labels (one for a scalar channel), for the chip's color
    /// bars and its hover legend. Never empty in practice -
    /// `build_channel_series` always emits at least one component - and the
    /// chip degrades to a plain one if it ever were.
    pub(super) components: Vec<String>,
}

/// The sorted union of channels across all series, with palette indices.
pub(super) fn loaded_channels<'a>(
    all_channels: impl Iterator<Item = &'a crate::series::ChannelSeries>,
) -> Vec<LoadedChannel> {
    let mut by_name: Vec<(&str, Option<&str>, Vec<String>)> = Vec::new();
    for channel in all_channels {
        let labels = || {
            channel
                .components
                .iter()
                .map(|c| c.label.clone())
                .collect::<Vec<String>>()
        };
        match by_name
            .iter_mut()
            .find(|(name, _, _)| *name == channel.name)
        {
            // The widest series wins: files may carry differing component
            // counts under one name, and the chip should show them all.
            Some((_, _, widest)) => {
                if channel.components.len() > widest.len() {
                    *widest = labels();
                }
            }
            None => by_name.push((&channel.name, channel.unit.as_deref(), labels())),
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

/// The chip groups always shown, in row order.
const BASIC_GROUPS: [&[MetricKind]; 2] = [
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

/// The Environment group: phenomena around the receiver, downloaded from an
/// archive for the days in view. The solar flare chip closes the group and
/// carries no [`MetricKind`], so it is not listed here.
const ENVIRONMENT_GROUP: &[MetricKind] = &[
    MetricKind::Jamming,
    MetricKind::Kp,
    MetricKind::Hp30,
    MetricKind::Tec,
];

/// The chip groups shown only while the advanced section is open.
const ADVANCED_GROUPS: [&[MetricKind]; 2] = [
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

/// Which data-backed metrics have values for the visible tracks. Their
/// chips stay visible and disabled without them, per DESIGN.md.
#[derive(Debug, Clone, Copy)]
pub(super) struct MetricAvailability {
    pub(super) snap_error: bool,
    pub(super) jamming: bool,
    pub(super) hp30: bool,
    pub(super) kp: bool,
    pub(super) tec: bool,
}

impl MetricAvailability {
    /// Whether `kind` has data behind it, which is what enables its chip and
    /// draws its line.
    pub(super) fn has_data(self, kind: MetricKind) -> bool {
        self.unavailable_hover(kind).is_none()
    }

    /// Why `kind`'s chip is disabled, or [`None`] when it has data.
    fn unavailable_hover(self, kind: MetricKind) -> Option<&'static str> {
        match kind {
            MetricKind::SnapError if !self.snap_error => Some(
                "No completed snap run for the visible tracks - run snap to road from the \
                 side panel first",
            ),
            MetricKind::Jamming if !self.jamming => {
                Some("No interference data is archived for these tracks' days")
            }
            MetricKind::Hp30 if !self.hp30 => {
                Some("No Hp30 values are archived for these tracks' days")
            }
            MetricKind::Kp if !self.kp => Some("No Kp values are archived for these tracks' days"),
            MetricKind::Tec if !self.tec => {
                Some("No TEC values are archived for these tracks' days")
            }
            _ => None,
        }
    }
}

/// The solar flare markers' chip: whether they draw, whether every flare's
/// span is shaded without hovering it, and whether the archive holds a flare
/// over the span the plot shows.
pub(super) struct FlareChipState<'a> {
    pub(super) visible: &'a mut bool,
    pub(super) always_show_spans: &'a mut bool,
    pub(super) available: bool,
}

/// Why the flare chip is disabled. Never hidden, per DESIGN.md.
const NO_ARCHIVED_FLARES: &str = "No solar flares are archived for the days in view";

/// The span-shading toggle in the flare chip's context menu.
const ALWAYS_SHOW_SPANS: &str = "Always show each flare's span";
const ALWAYS_SHOW_SPANS_HOVER: &str = "Shade every flare's active time";

/// The flare markers' toggle: the metric chip's look, with a context menu
/// holding the span-shading setting.
///
/// Returns whether the pointer is over the chip, which shades every visible
/// flare's span.
fn flare_chip(
    ui: &mut egui::Ui,
    FlareChipState {
        visible,
        always_show_spans,
        available,
    }: FlareChipState<'_>,
) -> bool {
    let color = gt_ui_theme::FLARE_M_CLASS.resolve(ui.visuals().dark_mode);
    if !available {
        disabled_chip(ui, gt_flare::text::LAYER_LABEL, color, NO_ARCHIVED_FLARES);
        return false;
    }
    let (fill, text_color) = if *visible {
        (color.gamma_multiply(0.75), Color32::WHITE)
    } else {
        (color.gamma_multiply(0.12), Color32::from_gray(100))
    };
    let chip = Button::new(
        RichText::new(gt_flare::text::LAYER_LABEL)
            .color(text_color)
            .small(),
    )
    .fill(fill)
    .corner_radius(4.0);
    let response = ui.add(chip);
    let hovered = response.hovered();
    response.context_menu(|ui| {
        ui.checkbox(always_show_spans, ALWAYS_SHOW_SPANS)
            .on_hover_text(ALWAYS_SHOW_SPANS_HOVER);
    });
    if ChipHover::Structured(&gt_flare::text::PLOT_HOVER)
        .attach(response)
        .clicked()
    {
        *visible = !*visible;
    }
    hovered
}

/// Which optional chip sections are revealed, gating their lines exactly as
/// the chips are gated.
#[derive(Clone, Copy)]
pub(super) struct SectionGates {
    pub(super) show_advanced: bool,
    pub(super) show_channels: bool,
}

/// A chip the pointer can rest on: a metric's, a loaded channel's (by name),
/// or the solar flares'. Drives the hover highlight that dims every other
/// line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HoveredChip {
    Metric(MetricKind),
    Channel(String),
    SolarFlare,
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
    present: ConstellationSet,
    kinds: &[MetricKind],
    show_advanced: bool,
    available: MetricAvailability,
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
        // A data-backed chip stays visible but disabled until its data
        // exists - never hidden, per DESIGN.md.
        if let Some(hover) = available.unavailable_hover(kind) {
            disabled_chip(
                ui,
                kind.label(),
                gt_ui_theme::metric_color(kind, dark_mode),
                hover,
            );
            continue;
        }
        let mut enabled = vis.field(kind);
        let (s, h) = metric_chip(
            ui,
            &mut enabled,
            kind.label(),
            gt_ui_theme::metric_color(kind, dark_mode),
            kind.hover(),
        );
        vis.set(kind, enabled);
        if s {
            *show_only = Some(kind);
        }
        if h {
            *hovered = Some(HoveredChip::Metric(kind));
        }
    }
}

/// The two section-reveal toggles: Advanced (always) and Channels (only when a
/// loaded track has channels). Both sections are hidden by default.
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
    component_colors: &mut FxHashMap<String, Vec<Option<Color32>>>,
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
            channel,
            component_colors,
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

/// The visibility of every series the chip row currently offers. The
/// show/hide-all button's icon states what a click on it changes: the icon
/// reads this struct and the click writes it.
struct ShownSeriesVisibility<'a> {
    metrics: &'a mut MetricVisibility,
    present: ConstellationSet,
    show_advanced: bool,
    /// Empty while the Channels section is collapsed.
    channels: &'a [LoadedChannel],
    channel_visibility: &'a mut ChannelVisibility,
    /// The flare markers' flag, [`None`] while the flare chip is disabled for
    /// want of an archived flare.
    solar_flares: Option<&'a mut bool>,
}

impl ShownSeriesVisibility<'_> {
    fn all_visible(&self) -> bool {
        self.metrics.all_enabled(self.present, self.show_advanced)
            && self
                .channels
                .iter()
                .all(|channel| self.channel_visibility.is_visible(&channel.name))
            && self.solar_flares.as_deref().is_none_or(|&visible| visible)
    }

    fn set_all_visible(&mut self, visible: bool) {
        self.metrics
            .set_all(visible, self.present, self.show_advanced);
        for channel in self.channels {
            self.channel_visibility.set(&channel.name, visible);
        }
        if let Some(solar_flares) = self.solar_flares.as_deref_mut() {
            *solar_flares = visible;
        }
    }
}

/// Draw the per-metric filter controls above the track plot.
///
/// All controls and metric chips share a single `horizontal_wrapped` row so they
/// fill available horizontal space before wrapping.
///
/// Returns the chip currently being hovered (a metric's, a channel's or the
/// solar flares'), or `None`. The caller passes this to `add_series_lines` to
/// highlight the hovered line and dim the rest, matching the standard
/// egui-plot legend hover behaviour.
#[expect(
    clippy::too_many_arguments,
    reason = "the filter row owns every plot toggle: grid/sync, the metric, channel and flare visibility, and both section gates"
)]
pub(super) fn metric_filter_row(
    ui: &mut egui::Ui,
    vis: &mut MetricVisibility,
    present: ConstellationSet,
    channels: &[LoadedChannel],
    channel_vis: &mut ChannelVisibility,
    component_colors: &mut FxHashMap<String, Vec<Option<Color32>>>,
    show_grid: &mut bool,
    line_width: &mut f32,
    sync_to_map: &mut bool,
    show_advanced: &mut bool,
    show_channels: &mut bool,
    available: MetricAvailability,
    flares: FlareChipState<'_>,
) -> Option<HoveredChip> {
    let mut show_only = None;
    let mut show_only_channel: Option<String> = None;
    let mut hovered_chip = None;

    ui.horizontal_wrapped(|ui| {
        // Instant tooltips: egui's default delay-with-grace makes the chip
        // hovers appear instantly when another tooltip was recently shown
        // (approaching from the plot below) but lag when coming from the
        // map above - consistent immediacy beats the inconsistency.
        ui.style_mut().interaction.tooltip_delay = 0.0;
        // Sync toggle.
        if ui
            .selectable_label(*sync_to_map, ICON_LINK)
            .on_hover_text(if *sync_to_map {
                "Syncing plot time range to map viewport. Click to disable."
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

        let mut shown_series = ShownSeriesVisibility {
            metrics: &mut *vis,
            present,
            show_advanced: *show_advanced,
            channels: if *show_channels { channels } else { &[] },
            channel_visibility: &mut *channel_vis,
            solar_flares: flares.available.then_some(&mut *flares.visible),
        };
        let all_visible = shown_series.all_visible();
        let eye_icon = if all_visible {
            ICON_EYE_SLASH
        } else {
            ICON_EYE
        };
        if ui
            .small_button(eye_icon)
            .on_hover_text(if all_visible {
                "Hide every metric, channel and flare marker"
            } else {
                "Show every metric, channel and flare marker"
            })
            .clicked()
        {
            shown_series.set_all_visible(!all_visible);
        }

        section_toggles(ui, show_advanced, show_channels, !channels.is_empty());

        // Basic groups, each separated by a divider.  Adding a new metric family
        // is just another `chip_group` call with its `MetricKind` slice.
        let basic_groups = BASIC_GROUPS;
        for group in basic_groups {
            chip_group(
                ui,
                vis,
                present,
                group,
                *show_advanced,
                available,
                &mut show_only,
                &mut hovered_chip,
            );
        }

        chip_group(
            ui,
            vis,
            present,
            ENVIRONMENT_GROUP,
            *show_advanced,
            available,
            &mut show_only,
            &mut hovered_chip,
        );
        if flare_chip(ui, flares) {
            hovered_chip = Some(HoveredChip::SolarFlare);
        }

        // Advanced groups, shown only when revealed.  Every kind here must report
        // `MetricKindUi::is_advanced() == true` so line drawing and the
        // show/hide-all scope stay consistent with these chips' visibility.
        if *show_advanced {
            let advanced_groups = ADVANCED_GROUPS;
            for group in advanced_groups {
                chip_group(
                    ui,
                    vis,
                    present,
                    group,
                    *show_advanced,
                    available,
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
                component_colors,
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
        vis.set(kind, true);
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

/// A chip rendered disabled: off-state visuals, no interaction, hover text
/// explaining what to do first.
fn disabled_chip(ui: &mut egui::Ui, name: &str, color: Color32, hover: &str) {
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
    hover: Option<ChipHover>,
) -> (bool, bool) {
    let (show_only, response) = chip_button(ui, enabled, name, color, |_| {});
    let hovered = response.hovered();
    if let Some(hover) = hover {
        hover.attach(response);
    }
    (show_only, hovered)
}

/// The shared chip widget behind [`metric_chip`] and [`channel_chip`]: the
/// toggle button and the show-only context menu, which `extend_menu` may
/// append to (the channel chip adds its color pickers there). Returns the
/// response so each caller attaches its own hover and [`channel_chip`] can
/// paint its bars over the rect.
fn chip_button(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    name: &str,
    color: Color32,
    extend_menu: impl FnOnce(&mut egui::Ui),
) -> (bool, egui::Response) {
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
    let mut show_only = false;
    response.context_menu(|ui| {
        if ui.button("Show only this").clicked() {
            show_only = true;
            ui.close();
        }
        extend_menu(ui);
    });
    (show_only, response)
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

/// Side of one color square in the chip's hover legend, in points.
const LEGEND_SQUARE_SIZE: f32 = 10.0;

/// A channel's chip: the metric chip extended with a bar strip along the
/// bottom edge, one bar per component in that component's line color - the
/// legend for a vector channel's x/y/z hues.
fn channel_chip(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    name: &str,
    color: Color32,
    channel: &LoadedChannel,
    component_colors: &mut FxHashMap<String, Vec<Option<Color32>>>,
) -> (bool, bool) {
    // The color pickers live in the right-click menu: a menu stays open while
    // its submenus are used.  A hover tooltip closes the moment the pointer
    // leaves the chip.
    let (show_only, response) = chip_button(ui, enabled, name, color, |ui| {
        for (index, label) in channel.components.iter().enumerate() {
            // The picker submenu only closes on a click outside (or the
            // menu items' explicit closes): a menu's default close-on-any-
            // click would dismiss the picker on a simple click, while a
            // click-drag survived - inconsistent mid-edit.
            let picker = egui::containers::menu::SubMenuButton::new(format!("Color of {label}"))
                .config(
                    egui::containers::menu::MenuConfig::new()
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                );
            picker.ui(ui, |ui| {
                let mut edited =
                    effective_component_color(component_colors, &channel.name, color, index);
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut edited,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    let overrides = component_colors
                        .entry(channel.name.clone())
                        .or_insert_with(|| vec![None; channel.components.len()]);
                    // Widen defensively: another file may have grown the
                    // component count since the overrides were stored.
                    if overrides.len() < channel.components.len() {
                        overrides.resize(channel.components.len(), None);
                    }
                    if let Some(slot) = overrides.get_mut(index) {
                        *slot = Some(edited);
                    }
                }
            });
        }
        let has_overrides = component_colors
            .get(&channel.name)
            .is_some_and(|colors| colors.iter().any(Option::is_some));
        if has_overrides && ui.button("Reset colors").clicked() {
            component_colors.remove(&channel.name);
            ui.close();
        }
    });
    let hovered = response.hovered();
    let rect = response.rect;
    response.on_hover_ui(|ui| {
        ui.label("Sensor channel recorded alongside the track");
        for (index, label) in channel.components.iter().enumerate() {
            ui.horizontal(|ui| {
                let (square, _) = ui.allocate_exact_size(
                    egui::Vec2::splat(LEGEND_SQUARE_SIZE),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(
                    square,
                    CHIP_BAR_CORNER_RADIUS,
                    effective_component_color(component_colors, &channel.name, color, index),
                );
                ui.label(label);
            });
        }
        ui.label(RichText::new("Right-click to pick colors").weak());
    });
    let Some(components) = NonZeroUsize::new(channel.components.len()) else {
        return (show_only, hovered);
    };
    let components = components.get();
    let strip = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - CHIP_BAR_HEIGHT),
        rect.max,
    );
    let bar_width =
        (strip.width() - CHIP_BAR_GAP * (components.saturating_sub(1)) as f32) / components as f32;
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
            effective_component_color(component_colors, &channel.name, color, index)
                .gamma_multiply(alpha),
        );
    }
    (show_only, hovered)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::plot_widget::style::CHANNEL_PALETTE;

    /// Every per-constellation metric maps to a constellation and every
    /// all-constellation metric maps to `None`, with the two groups together
    /// covering all `MetricKind::COUNT` variants.
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
        // 6 constellations x {seen, fix, utilization rate, slip}.
        assert_eq!(with, 24);
    }

    /// The default set is the one each metric declares, and toggling one
    /// leaves the rest untouched.
    #[test]
    fn visibility_defaults_per_metric_and_toggles_independently() {
        let mut vis = MetricVisibility::default();
        assert!(MetricKind::iter().all(|k| vis.field(k) == k.visible_by_default()));

        vis.set(MetricKind::Velocity, false);
        assert!(!vis.field(MetricKind::Velocity));
        assert!(
            MetricKind::iter()
                .filter(|&k| k != MetricKind::Velocity)
                .all(|k| vis.field(k) == k.visible_by_default())
        );

        vis.set(MetricKind::Velocity, true);
        assert!(MetricKind::iter().all(|k| vis.field(k) == k.visible_by_default()));
    }

    /// Each metric has independent visibility state, so toggling one never
    /// disturbs another (a shared bit would show up as cross-talk here).
    #[test]
    fn each_metric_toggles_independently() {
        for target in MetricKind::iter() {
            let mut vis = MetricVisibility::default();
            vis.set(target, !target.visible_by_default());
            assert_eq!(vis.field(target), !target.visible_by_default());
            assert!(
                MetricKind::iter()
                    .filter(|&k| k != target)
                    .all(|k| vis.field(k) == k.visible_by_default()),
                "toggling {target:?} disturbed another metric"
            );
        }
    }

    /// A per-constellation chip/line shows only when its constellation appears
    /// in the data.  All-constellation metrics always show (subject to the
    /// advanced gate).  This is the rule that hides empty NavIC/QZSS chips.
    #[test]
    fn metric_is_shown_gates_on_presence_and_advanced() {
        let none = ConstellationSet::empty();
        let gps_only = ConstellationSet::single(Constellation::Gps);

        // Totals always show regardless of which constellations are present.
        assert!(metric_is_shown(MetricKind::SatsSeen, none, false));
        // GPS chip hidden with no data, shown once GPS is present.
        assert!(!metric_is_shown(MetricKind::GpsSeen, none, false));
        assert!(metric_is_shown(MetricKind::GpsSeen, gps_only, false));
        // NavIC/QZSS stay hidden in a GPS-only recording.
        assert!(!metric_is_shown(MetricKind::NavicSeen, gps_only, false));
        assert!(!metric_is_shown(MetricKind::QzssFix, gps_only, false));
        // Advanced metrics need the advanced section open *and* presence.
        assert!(!metric_is_shown(MetricKind::UtilGps, gps_only, false));
        assert!(metric_is_shown(MetricKind::UtilGps, gps_only, true));
        assert!(!metric_is_shown(MetricKind::UtilNavic, gps_only, true));
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

        let channel = |name: &str, unit: Option<&str>, components: usize| ChannelSeries {
            name: name.to_owned(),
            unit: unit.map(str::to_owned),
            components: (0..components)
                .map(|i| ChannelComponentSeries {
                    label: format!("{name}.{i}"),
                    runs: Vec::new(),
                })
                .collect(),
            backward_time_steps: Vec::new(),
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
            channels[0].components,
            ["accel.0", "accel.1", "accel.2"],
            "the widest series' component labels win"
        );
        // Palette indices follow the sorted order, so a channel keeps one hue
        // across files.
        assert_eq!(channels[0].color_index, 0);
        assert_eq!(channels[1].color_index, 1);
        assert_eq!(channel_color(0), channel_color(CHANNEL_PALETTE.len()));
    }

    /// Every metric in the Environment group hovers with the three scannable
    /// lines, not a paragraph of prose.
    #[test]
    fn every_environment_metric_hovers_with_scannable_lines() {
        assert!(
            ENVIRONMENT_GROUP
                .iter()
                .all(|&kind| matches!(kind.hover(), Some(ChipHover::Structured(_)))),
            "an environment chip still hovers with prose"
        );
    }

    /// Each environment chip closes its hover on its own reference document,
    /// in the one phrasing all five share.
    #[rstest]
    #[case::interference(
        &gt_jam::text::PLOT_HOVER,
        "More: 'How does aircraft interference data relate to GNSS?' in Settings."
    )]
    #[case::kp(
        GeomagneticIndex::Kp.plot_hover(),
        "More: 'How does geomagnetic activity affect GNSS?' in Settings."
    )]
    #[case::hp30(
        GeomagneticIndex::Hp30.plot_hover(),
        "More: 'How does geomagnetic activity affect GNSS?' in Settings."
    )]
    #[case::tec(
        &gt_ionex::text::PLOT_HOVER,
        "More: 'How does ionospheric TEC affect GNSS?' in Settings."
    )]
    #[case::solar_flares(
        &gt_flare::text::PLOT_HOVER,
        "More: 'How do solar flares affect GNSS?' in Settings."
    )]
    fn every_environment_chip_hover_points_at_its_reference_document(
        #[case] hover: &MetricChipHover,
        #[case] expected_reference_line: &str,
    ) {
        assert_eq!(hover.reference_line(), expected_reference_line);
    }

    /// Every metric must appear in exactly one chip group. A metric wired
    /// into `label`, `hover` and the line renderer but left out of the
    /// groups draws with no chip to discover or toggle it.
    #[test]
    fn every_metric_has_exactly_one_chip() {
        let mut seen: Vec<MetricKind> = BASIC_GROUPS
            .into_iter()
            .chain(ADVANCED_GROUPS)
            .chain([ENVIRONMENT_GROUP])
            .flatten()
            .copied()
            .collect();
        let total = seen.len();
        seen.sort_by_key(|kind| *kind as usize);
        seen.dedup();
        assert_eq!(total, seen.len(), "a metric is listed in two groups");

        let missing: Vec<MetricKind> = MetricKind::iter()
            .filter(|kind| !seen.contains(kind))
            .collect();
        assert!(missing.is_empty(), "no chip for {missing:?}");
        assert_eq!(seen.len(), <MetricKind as strum::EnumCount>::COUNT);
    }
}
