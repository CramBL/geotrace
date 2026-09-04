use std::fmt;

use gt_types::TrackRef;
use gt_ui_types::{PinWithheld, PinnedPopup};

use crate::classify::PointClass;

/// Space between the widest label and the glyph column.
const LABEL_GAP: usize = 2;

/// Marker for a point covered by the hovered results-table match.
const HOVER_MARKER: char = '~';

/// Marker for the point whose popup is pinned.
const SELECT_MARKER: char = '^';

/// One track's points as glyphs, plus the sparse annotation rows.
pub struct TrackPicture {
    pub track: TrackRef,
    pub label: String,
    pub points: Vec<PointClass>,
}

impl TrackPicture {
    fn glyphs(&self) -> String {
        self.points.iter().map(PointClass::glyph).collect()
    }

    fn markers(&self, pick: impl Fn(&PointClass) -> bool, marker: char) -> Option<String> {
        let row: String = self
            .points
            .iter()
            .map(|point| if pick(point) { marker } else { ' ' })
            .collect();
        (!row.trim().is_empty()).then(|| row.trim_end().to_owned())
    }
}

/// What the map shows right now: every point of every track as one glyph, the
/// selection and hovered match beneath it, and the real map's own element
/// counts.
///
/// Rendered as a picture of the tracks, so a scenario's snapshot shows the
/// whole result in one block:
///
/// ```text
/// track.gtd#0  ..0000xx
///      select      ^
/// popup: drawn
/// counts: shown 6, halos 1
/// ```
pub struct MapPicture {
    pub tracks: Vec<TrackPicture>,
    /// What the pinned popup does, from [`gt_ui_types::MapHighlight::pin_this_frame`].
    /// `None` when nothing is pinned, which is when the line is left out.
    pub pin: Option<PinnedPopup>,
    pub stale: bool,
    /// Points the map draws, from [`gt_map::display_counts::DisplayCounts`].
    pub shown_points: usize,
    /// Matches with any drawn point, from the same counts.
    pub halos: usize,
}

impl fmt::Display for MapPicture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let width = self
            .tracks
            .iter()
            .map(|track| track.label.chars().count())
            .chain(std::iter::once(SELECT_LABEL.chars().count()))
            .max()
            .unwrap_or(0);
        for track in &self.tracks {
            writeln!(
                f,
                "{:>width$}{:LABEL_GAP$}{}",
                track.label,
                "",
                track.glyphs()
            )?;
            for (label, row) in [
                (
                    HOVER_LABEL,
                    track.markers(|point| point.hover_matched, HOVER_MARKER),
                ),
                (
                    SELECT_LABEL,
                    track.markers(|point| point.selected, SELECT_MARKER),
                ),
            ] {
                if let Some(row) = row {
                    writeln!(f, "{label:>width$}{:LABEL_GAP$}{row}", "")?;
                }
            }
        }
        // Omitted when nothing is pinned.
        if let Some(pin) = self.pin {
            writeln!(f, "popup: {}", popup_line(pin))?;
        }
        write!(
            f,
            "counts: shown {}, halos {}",
            self.shown_points, self.halos
        )?;
        if self.stale {
            write!(f, "\nstale")?;
        }
        Ok(())
    }
}

const HOVER_LABEL: &str = "hover";
const SELECT_LABEL: &str = "select";

/// The pinned popup as one line: drawn, or withheld with the reason the map
/// withheld it.
fn popup_line(pin: PinnedPopup) -> String {
    match pin {
        PinnedPopup::Drawn(_) => "drawn".to_owned(),
        PinnedPopup::Withheld { reason, .. } => format!("withheld ({})", withheld_reason(reason)),
    }
}

fn withheld_reason(reason: PinWithheld) -> &'static str {
    match reason {
        PinWithheld::HiddenByQuery => "hidden by the query",
        PinWithheld::OutsideTimeFilter => "outside the time filter",
        PinWithheld::TrackNotShown => "track not shown",
        PinWithheld::CategoryHidden => "category hidden",
    }
}

#[cfg(test)]
mod tests {
    use gt_ui_types::{DrawLayerMask, PointVisibility};

    use super::*;
    use crate::dataset::track;

    fn shown(layer: Option<usize>) -> PointClass {
        let mut draw_layers = DrawLayerMask::default();
        if let Some(layer) = layer {
            draw_layers.insert(layer);
        }
        PointClass {
            visibility: PointVisibility::Shown,
            draw_layers,
            hover_matched: false,
            selected: false,
        }
    }

    #[test]
    fn a_picture_aligns_the_annotation_rows_under_the_glyphs() {
        let mut points = vec![shown(None), shown(Some(0)), shown(Some(0)), shown(None)];
        if let Some(point) = points.get_mut(1) {
            point.hover_matched = true;
        }
        if let Some(point) = points.get_mut(2) {
            point.hover_matched = true;
            point.selected = true;
        }
        let picture = MapPicture {
            tracks: vec![TrackPicture {
                track: track(0, 0),
                label: "a.gtd#0".to_owned(),
                points,
            }],
            pin: None,
            stale: false,
            shown_points: 4,
            halos: 1,
        };
        insta::assert_snapshot!(picture, @"
        a.gtd#0  .00.
          hover   ~~
         select    ^
        counts: shown 4, halos 1
        ");
    }

    #[test]
    fn staleness_reads_as_its_own_line() {
        let picture = MapPicture {
            tracks: vec![TrackPicture {
                track: track(0, 0),
                label: "a.gtd#0".to_owned(),
                points: vec![shown(None), shown(None)],
            }],
            pin: None,
            stale: true,
            shown_points: 2,
            halos: 0,
        };
        insta::assert_snapshot!(picture, @"
        a.gtd#0  ..
        counts: shown 2, halos 0
        stale
        ");
    }
}
