use egui::{Label, Role, WidgetInfo};
use gt_loaded_files::RecordingNames;
use gt_session_bridge::QueryJobId;
use gt_types::{LoadedFile, PointIdx, TrackRef};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(super) enum PresentationDetail {
    Point(PointIdx),
    Points { start: usize, end: usize },
    QueryMatch { query: usize, match_ordinal: usize },
}

#[derive(Clone, Copy)]
pub(super) struct PresentationQueryMatch {
    pub(super) job: QueryJobId,
    pub(super) query: usize,
    pub(super) match_ordinal: usize,
}

impl PresentationQueryMatch {
    pub(super) fn detail(self) -> PresentationDetail {
        PresentationDetail::QueryMatch {
            query: self.query,
            match_ordinal: self.match_ordinal,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PanePresentation {
    pub(super) track: TrackRef,
    pub(super) detail: PresentationDetail,
}

impl PanePresentation {
    pub(super) fn label(self, names: &RecordingNames, files: &[LoadedFile]) -> Option<String> {
        let name = names.display_name(files, self.track.fi)?;
        let track = self.track.resolve(files)?.metadata.index;
        let detail = match self.detail {
            PresentationDetail::Point(point) => format!("Point {}", point.as_usize()),
            PresentationDetail::Points { start, end } if start + 1 == end => {
                format!("Point {start}")
            }
            PresentationDetail::Points { start, end } => format!("Points {start} to {}", end - 1),
            PresentationDetail::QueryMatch {
                query,
                match_ordinal,
            } => {
                format!("Query {} match {}", query + 1, match_ordinal + 1)
            }
        };
        Some(format!("{name} / Track {track} / {detail}"))
    }
}

pub(super) struct PresentationControls<'a> {
    pub(super) id: Uuid,
    pub(super) caption: &'a str,
    pub(super) dismiss_label: &'static str,
}

impl PresentationControls<'_> {
    pub(super) fn show(self, ui: &mut egui::Ui) -> bool {
        ui.push_id(self.id, |ui| {
            let dismiss = ui.small_button("Dismiss").on_hover_text(self.dismiss_label);
            dismiss.widget_info(|| {
                WidgetInfo::labeled(Role::Button, dismiss.enabled(), self.dismiss_label)
            });
            ui.add(Label::new(self.caption).truncate())
                .on_hover_text(self.caption);
            dismiss.clicked()
        })
        .inner
    }
}
