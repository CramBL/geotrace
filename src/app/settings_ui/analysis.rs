//! Detection parameters the plot applies to loaded data as they are edited.

use egui::{DragValue, Grid};
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::WAVE_SINE as ICON_WAVE_SINE;

use crate::app::App;
use crate::app::settings_ui::SettingsPage;

const ELEVATION_MASK_LABEL: &str = "Elevation mask";
const SNR_DROP_THRESHOLD_LABEL: &str = "SNR drop threshold";
const SLIP_WINDOW_LABEL: &str = "Slip window";
/// The Processing page's generated-marker row for the same excursions carries
/// this label too, and reads the threshold from this page.
pub(super) const CLOCK_OFFSET_EXCURSION_LABEL: &str = "Clock offset excursion";
const MARK_MASKED_SATELLITES_LABEL: &str = "Mark masked-out used satellites";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    ELEVATION_MASK_LABEL,
    SNR_DROP_THRESHOLD_LABEL,
    SLIP_WINDOW_LABEL,
    CLOCK_OFFSET_EXCURSION_LABEL,
    MARK_MASKED_SATELLITES_LABEL,
];

impl App {
    pub(super) fn show_analysis_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Analysis.show_header(ui);
        Grid::new("analysis_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                // Start from the analysis config the plot currently uses,
                // then live-apply any edits below.
                let mut analysis = self.shared.borrow().plot_state.analysis;

                let mask_help =
                    "Satellites below this elevation are excluded from the 'in view' \
                     baseline of the utilization rate, and a satellite must sit above it \
                     to count toward the slip rate. This keeps the receiver from being \
                     penalised for ignoring low-elevation satellites (high atmospheric \
                     delay and multipath). \
                     For example, a 0° mask counts horizon satellites the receiver \
                     naturally rejects, which lowers the utilization rate and inflates the \
                     slip rate with routine horizon fades; a 30° mask considers only \
                     high, clean satellites.";
                ui.label(format!("{ICON_FUNNEL} {ELEVATION_MASK_LABEL}"))
                    .on_hover_text(mask_help);
                ui.add(
                    DragValue::new(&mut analysis.elevation_mask_deg)
                        .range(0.0..=90.0)
                        .speed(1.0)
                        .fixed_decimals(0)
                        .suffix("°"),
                )
                .on_hover_text(mask_help);
                ui.end_row();

                let snr_help =
                    "An above-mask satellite whose SNR falls by more than this many dB-Hz \
                     between consecutive epochs is counted as a loss-of-lock (slip), a \
                     sign of a momentary signal break rather than ordinary variation. \
                     For example, a low threshold like 3 dB-Hz flags ordinary signal \
                     fluctuation as slips and inflates the rate; a high threshold like \
                     25 dB-Hz reports only near-total dropouts and may miss real slips.";
                ui.label(format!("{ICON_WAVE_SINE} {SNR_DROP_THRESHOLD_LABEL}"))
                    .on_hover_text(snr_help);
                ui.add(
                    DragValue::new(&mut analysis.snr_drop_db)
                        .range(1.0..=60.0)
                        .speed(0.5)
                        .fixed_decimals(0)
                        .suffix(" dB-Hz"),
                )
                .on_hover_text(snr_help);
                ui.end_row();

                let window_help =
                    "Trailing window over which the slip rate is averaged, in minutes. \
                     The plotted value is the slips counted in the window divided by its \
                     length, in slips per minute.";
                ui.label(format!("{ICON_CLOCK} {SLIP_WINDOW_LABEL}"))
                    .on_hover_text(window_help);
                ui.add(
                    DragValue::new(&mut analysis.slip_window_min)
                        .range(1.0..=120.0)
                        .speed(1.0)
                        .fixed_decimals(0)
                        .suffix(" min"),
                )
                .on_hover_text(window_help);
                ui.end_row();

                let excursion_help =
                    "A sample whose GPS/system clock offset sits more than this far from \
                     the track's own baseline offset is treated as a clock offset \
                     excursion: it is kept off the clock offset line, so one sample \
                     carrying a whole recording gap cannot flatten the shared y-axis, and \
                     marked at the edge of the view instead. Hover a marker for its real \
                     offset. A large but steady offset sits at the baseline and is never \
                     an excursion, and an offset that steps and stays is a clock \
                     discontinuity, not an excursion.";
                ui.label(format!("{ICON_WARNING} {CLOCK_OFFSET_EXCURSION_LABEL}"))
                    .on_hover_text(excursion_help);
                ui.add(
                    DragValue::new(&mut analysis.clock_excursion_threshold_s)
                        .range(gt_plot::CLOCK_EXCURSION_THRESHOLD_RANGE_S)
                        .speed(1.0)
                        .fixed_decimals(0)
                        .suffix(" s"),
                )
                .on_hover_text(excursion_help);
                ui.end_row();

                // Live-apply: re-derives only the analysis-dependent
                // series, and keeps the loader in step for files loaded
                // later. `set_analysis` is a no-op when unchanged.
                self.loader.analysis_config = analysis;
                // Slip markers share these detection params, but as
                // load-time generated markers they only pick up the
                // change on the next load or "Apply to loaded data".
                self.processing_config
                    .generated_markers
                    .slip_elevation_mask_deg = analysis.elevation_mask_deg;
                self.processing_config.generated_markers.slip_snr_drop_db = analysis.snr_drop_db;
                self.processing_config
                    .generated_markers
                    .clock_excursion_threshold_s = analysis.clock_excursion_threshold_s;
                {
                    let mut shared = self.shared.borrow_mut();
                    let s = &mut *shared;
                    s.plot_state.set_analysis(&s.loaded_files, analysis);
                }

                let mark_help =
                    "Place a red cross on the plot at each epoch where the receiver used a \
                     satellite below the elevation mask. Hover a marker to see which \
                     satellites and their elevation. Such satellites are excluded from the \
                     utilization rate, and the marker keeps that exclusion visible. Markers \
                     show with the 'Util all' plot.";
                ui.label(format!("{ICON_WARNING} {MARK_MASKED_SATELLITES_LABEL}"))
                    .on_hover_text(mark_help);
                let mut mark = self.shared.borrow().plot_state.mark_masked_fix;
                if ui.checkbox(&mut mark, "").on_hover_text(mark_help).changed() {
                    self.shared.borrow_mut().plot_state.mark_masked_fix = mark;
                }
                ui.end_row();
            });
    }
}
