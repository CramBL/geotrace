//! The recording-name template setting: the text field and the guide popup that
//! shows while it has keyboard focus.

use egui::{Grid, Label, Popup, RichText};
use egui_phosphor::regular::TAG as ICON_TAG;
use gt_fmt::{EM_DASH, NameFields, Token, render_name_template};
use gt_loaded_files::LoadedFileEntry;
use gt_store::RecordingEntry;
use strum::IntoEnumIterator as _;

/// Field values for the structure preview: every token resolves to its own name.
const STRUCTURE_FIELDS: NameFields<'static> = NameFields {
    title: Some("title"),
    device: Some("device"),
    identity: Some("identity"),
    filename: "filename",
};

const GUIDE_WIDTH: f32 = 400.0;

/// Where the recording behind the preview line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewOrigin {
    LoadedFile,
    History,
}

/// The name fields of one recording, rendered through the template the user is
/// editing so the popup can show the outcome on real metadata.
#[derive(Debug, Clone)]
pub struct TemplatePreviewRecording {
    origin: PreviewOrigin,
    title: Option<String>,
    device: Option<String>,
    identity: Option<String>,
    filename: String,
}

impl TemplatePreviewRecording {
    /// Take the fields from a loaded file. The `{filename}` token gets the plain
    /// file name: with one recording in the preview there is no shared directory
    /// prefix to strip, unlike [`gt_loaded_files::RecordingNames`].
    pub fn from_loaded_file(entry: LoadedFileEntry<'_>) -> Self {
        let metadata = &entry.file().metadata;
        let filename = metadata
            .filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(metadata.filename.as_str());
        Self {
            origin: PreviewOrigin::LoadedFile,
            title: metadata.title.clone(),
            device: metadata.device.clone(),
            identity: entry
                .identity()
                .map(|id| gt_loaded_files::display_identity(id).0.to_owned()),
            filename: filename.to_owned(),
        }
    }

    /// Take the fields from a stored recording. History keeps no file name of its
    /// own: opening a recording from it names the file after the identity, and
    /// the preview follows that.
    pub fn from_history_entry(entry: &RecordingEntry) -> Self {
        let identity = gt_loaded_files::display_identity(&entry.db_ref.identity)
            .0
            .to_owned();
        Self {
            origin: PreviewOrigin::History,
            title: entry.title.clone(),
            device: entry.device.clone(),
            identity: Some(identity.clone()),
            filename: identity,
        }
    }

    fn render(&self, template: &str) -> String {
        render_name_template(
            template,
            &NameFields {
                title: self.title.as_deref(),
                device: self.device.as_deref(),
                identity: self.identity.as_deref(),
                filename: &self.filename,
            },
        )
    }

    fn source_text(&self) -> String {
        match self.origin {
            PreviewOrigin::LoadedFile => format!("From the loaded recording {}", self.filename),
            PreviewOrigin::History => format!("From the history recording {}", self.filename),
        }
    }
}

/// Show the template field, and the guide popup for as long as it has focus.
///
/// Returns `true` in the frame the user edited `template`.
pub fn recording_name_template_ui(
    ui: &mut egui::Ui,
    template: &mut String,
    preview: Option<&TemplatePreviewRecording>,
) -> bool {
    let label = ui
        .label(format!("{ICON_TAG} Recording name"))
        .on_hover_text("Template for the name shown for each recording in the side panel");
    let response = ui.text_edit_singleline(template).labelled_by(label.id);
    show_template_guide(&response, template, preview);
    response.changed()
}

fn show_template_guide(
    response: &egui::Response,
    template: &str,
    preview: Option<&TemplatePreviewRecording>,
) {
    Popup::from_response(response)
        .open(response.has_focus())
        .width(GUIDE_WIDTH)
        .gap(4.0)
        .show(|ui| {
            Grid::new("recording_name_template_guide")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Tokens").weak());
                    ui.label(
                        Token::iter()
                            .map(|token| format!("{{{token}}}"))
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Example").weak());
                    ui.label(format!("{{title}} {EM_DASH} {{device}}"));
                    ui.end_row();

                    ui.label(RichText::new("Structure").weak())
                        .on_hover_text("Your template with every token filled by its own name");
                    ui.label(render_name_template(template, &STRUCTURE_FIELDS));
                    ui.end_row();

                    ui.label(RichText::new("Preview").weak())
                        .on_hover_text("Your template on a real recording");
                    match preview {
                        Some(recording) => {
                            ui.label(recording.render(template))
                                .on_hover_text(recording.source_text());
                        }
                        None => {
                            ui.add_enabled(false, Label::new("No recording loaded or in history"));
                        }
                    }
                    ui.end_row();
                });
            ui.label(
                RichText::new(
                    "A token with no value is dropped, and so is the separator beside it",
                )
                .weak(),
            );
        });
}

#[cfg(test)]
mod tests {
    use gt_fmt::{Token, render_name_template};
    use strum::IntoEnumIterator as _;

    use super::STRUCTURE_FIELDS;

    /// Every token stands for its own name in the structure preview, so a token
    /// renamed in `gt-fmt` cannot leave the preview rendering the old name.
    #[test]
    fn structure_fields_mirror_the_token_names() {
        for token in Token::iter() {
            let name = token.to_string();
            assert_eq!(
                render_name_template(&format!("{{{name}}}"), &STRUCTURE_FIELDS),
                name
            );
        }
    }
}
