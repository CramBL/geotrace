use std::ops::Range;

use egui::Pos2;

use super::super::LineTableRows;
use super::super::selection_document::{Caret, DocumentRevision};
use super::CaretRange;
use super::keyboard::{NavigationEndpoint, SelectionMotion, VerticalMotion, VerticalResult};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HorizontalPosition(pub(super) f32);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum SelectionUnit {
    Character,
    Line,
    Word,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum AnchorMode {
    Extend,
    Replace,
}

#[derive(Clone, Copy)]
pub(super) enum PointerIntent {
    Drag(Pos2),
    Press {
        position: Pos2,
        time: f64,
        anchor: AnchorMode,
    },
    Release {
        position: Pos2,
        time: f64,
    },
}

#[derive(Clone, Copy)]
pub(super) struct GestureLimits {
    pub(super) distance: f32,
    pub(super) duration: f64,
    pub(super) multi_click_delay: f64,
}

#[derive(Clone, Copy)]
pub(super) struct SelectionTarget {
    pub(super) caret: Caret,
    pub(super) range: CaretRange,
}

impl SelectionTarget {
    fn belongs_to(&self, document: Option<DocumentRevision>) -> bool {
        Some(self.caret.revision()) == document
            && self.range.anchor().revision() == self.caret.revision()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum PointerCompletion {
    Caret,
    Selection,
}

#[derive(Default)]
pub(in crate::app::log_viewer) struct TableTextSelection {
    document: Option<DocumentRevision>,
    selected: Option<SelectedText>,
    last_click: Option<CompletedClick>,
    settled: Option<Range<Caret>>,
}

impl TableTextSelection {
    pub(in crate::app::log_viewer) fn replace_document(
        &mut self,
        document: Option<DocumentRevision>,
    ) {
        if self.document != document {
            *self = Self {
                document,
                ..Self::default()
            };
        }
    }

    pub(super) fn apply_pointer(
        &mut self,
        intent: PointerIntent,
        limits: GestureLimits,
        target: impl FnOnce(SelectionUnit, Pos2) -> Option<SelectionTarget>,
    ) -> Option<PointerCompletion> {
        match intent {
            PointerIntent::Press {
                position,
                time,
                anchor,
            } => {
                let unit = self
                    .last_click
                    .as_ref()
                    .map_or(SelectionUnit::Character, |click| {
                        click.next_unit(position, time, limits)
                    });
                let target =
                    target(unit, position).filter(|target| target.belongs_to(self.document))?;
                self.begin(
                    target,
                    SelectionGesture {
                        unit,
                        origin: position,
                        start_time: time,
                        anchor,
                        within_click_distance: true,
                    },
                );
            }
            PointerIntent::Drag(position) => self.extend(position, limits.distance, target),
            PointerIntent::Release { position, time } => {
                self.extend(position, limits.distance, target);
                return self.complete(position, time, limits);
            }
        }
        None
    }

    pub(super) fn clear(&mut self) {
        self.selected = None;
        self.last_click = None;
    }

    pub(super) fn stop_drag(&mut self) {
        if let Some(text) = &mut self.selected {
            text.gesture = None;
        }
    }

    pub(super) fn select_all(&mut self, document: &LineTableRows) {
        if self.document == Some(document.document_revision())
            && let Some(range) = document.full_range()
        {
            self.selected = Some(SelectedText {
                anchor: CaretRange::point(range.anchor()),
                range,
                gesture: None,
                preferred_x: None,
            });
        }
    }

    pub(super) fn move_endpoint(
        &mut self,
        motion: SelectionMotion,
        document: &LineTableRows,
        vertical_endpoint: impl FnOnce(VerticalMotion) -> Option<VerticalResult>,
    ) -> Option<NavigationEndpoint> {
        let text = self
            .selected
            .as_mut()
            .filter(|text| text.gesture.is_none())?;
        let previous = NavigationEndpoint {
            caret: text.range.endpoint(),
            preferred_x: text.preferred_x,
        };
        let endpoint = previous.moved(motion, document, vertical_endpoint)?;
        text.anchor = CaretRange::point(text.range.anchor());
        text.range = text.range.with_endpoint(endpoint.caret);
        text.preferred_x = endpoint.preferred_x;
        self.last_click = None;
        Some(endpoint)
    }

    pub(super) fn settle(&mut self) -> Option<CaretRange> {
        let selected = self.copy_range();
        let range = selected.map(CaretRange::ordered);
        if self.settled == range {
            return None;
        }
        self.settled = range;
        selected
    }

    pub(super) fn dragging(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|text| text.gesture.is_some())
    }

    pub(super) fn range(&self) -> Option<CaretRange> {
        self.selected.as_ref().map(|text| text.range)
    }

    pub(super) fn copy_range(&self) -> Option<CaretRange> {
        self.range().filter(|range| !range.ordered().is_empty())
    }

    fn begin(&mut self, target: SelectionTarget, gesture: SelectionGesture) {
        let existing = self
            .selected
            .take()
            .filter(|_| gesture.anchor == AnchorMode::Extend);
        let mut text = existing.unwrap_or(SelectedText {
            anchor: target.range,
            range: target.range,
            gesture: None,
            preferred_x: None,
        });
        text.preferred_x = None;
        text.gesture = Some(gesture);
        text.extend(target);
        self.selected = Some(text);
    }

    fn extend(
        &mut self,
        position: Pos2,
        max_click_distance: f32,
        target: impl FnOnce(SelectionUnit, Pos2) -> Option<SelectionTarget>,
    ) {
        let Some(text) = &mut self.selected else {
            return;
        };
        let Some(gesture) = &mut text.gesture else {
            return;
        };
        gesture.within_click_distance &= gesture.origin.distance(position) <= max_click_distance;
        if let Some(target) =
            target(gesture.unit, position).filter(|target| target.belongs_to(self.document))
        {
            text.extend(target);
        }
    }

    fn complete(
        &mut self,
        position: Pos2,
        time: f64,
        limits: GestureLimits,
    ) -> Option<PointerCompletion> {
        let text = self.selected.as_mut()?;
        let gesture = text.gesture.take()?;
        let selecting_characters =
            gesture.unit == SelectionUnit::Character && !text.range.ordered().is_empty();
        self.last_click = (gesture.within_click_distance
            && time - gesture.start_time <= limits.duration
            && !selecting_characters)
            .then_some(CompletedClick {
                position,
                time,
                unit: gesture.unit,
            });
        Some(
            if selecting_characters
                || gesture.unit != SelectionUnit::Character
                || gesture.anchor == AnchorMode::Extend
            {
                PointerCompletion::Selection
            } else {
                PointerCompletion::Caret
            },
        )
    }
}

/// egui reports the click count on release.
struct CompletedClick {
    position: Pos2,
    time: f64,
    unit: SelectionUnit,
}

impl CompletedClick {
    fn next_unit(&self, position: Pos2, time: f64, limits: GestureLimits) -> SelectionUnit {
        if time - self.time >= limits.multi_click_delay
            || position.distance(self.position) >= limits.distance
        {
            SelectionUnit::Character
        } else if self.unit == SelectionUnit::Character {
            SelectionUnit::Word
        } else {
            SelectionUnit::Line
        }
    }
}

struct SelectionGesture {
    unit: SelectionUnit,
    origin: Pos2,
    start_time: f64,
    anchor: AnchorMode,
    within_click_distance: bool,
}

struct SelectedText {
    anchor: CaretRange,
    range: CaretRange,
    gesture: Option<SelectionGesture>,
    preferred_x: Option<HorizontalPosition>,
}

impl SelectedText {
    fn extend(&mut self, target: SelectionTarget) {
        let anchor = self.anchor.ordered();
        let range = target.range.ordered();
        let pointer = target.caret;
        self.range =
            if pointer < anchor.start || (pointer == anchor.start && range.start < anchor.start) {
                CaretRange::point(anchor.end).with_endpoint(range.start)
            } else if pointer > anchor.end
                || anchor.is_empty()
                || (pointer == anchor.end && range.end > anchor.end)
            {
                CaretRange::point(anchor.start).with_endpoint(range.end)
            } else {
                self.anchor
            };
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests as line_table_tests;
    use super::super::keyboard::Direction;
    use super::*;

    #[test]
    fn selection_preserves_the_range_only_within_its_document() {
        let log = line_table_tests::source_log("2026-01-01 14:02:11 hello\n");
        let rows = LineTableRows::of(&log);
        let mut selection = TableTextSelection::default();
        selection.replace_document(Some(rows.document_revision()));
        selection.select_all(&rows);

        selection.replace_document(Some(rows.document_revision()));
        assert!(selection.copy_range().is_some());

        selection.replace_document(Some(DocumentRevision::next()));
        assert!(selection.copy_range().is_none());
    }

    #[test]
    fn row_motion_updates_the_caret_and_preferred_position_together() {
        let log =
            line_table_tests::source_log("2026-01-01 14:02:11 a\n2026-01-01 14:02:12 abcdef\n");
        let document = LineTableRows::of(&log);
        let mut selection = TableTextSelection::default();
        selection.replace_document(Some(document.document_revision()));
        selection.select_all(&document);
        let end = document.full_range().unwrap().endpoint();
        let short = document
            .document_row(end.row() - 1)
            .unwrap()
            .range()
            .unwrap()
            .endpoint();
        let column = HorizontalPosition(32.0);
        let up = SelectionMotion::Row(Direction::Backward);
        let down = SelectionMotion::Row(Direction::Forward);

        let missed = selection
            .move_endpoint(up, &document, |motion| {
                assert_eq!(motion.previous, end);
                assert_eq!(motion.preferred_x, None);
                Some(VerticalResult {
                    caret: None,
                    preferred_x: column,
                })
            })
            .unwrap();
        assert_eq!(missed.caret, end);
        assert_eq!(missed.preferred_x, Some(column));

        let on_short_row = selection
            .move_endpoint(up, &document, |motion| {
                assert_eq!(motion.preferred_x, Some(column));
                Some(VerticalResult {
                    caret: Some(short),
                    preferred_x: column,
                })
            })
            .unwrap();
        assert_eq!(on_short_row.caret, short);

        let on_last_row = selection
            .move_endpoint(down, &document, |motion| {
                assert_eq!(motion.previous, short);
                assert_eq!(motion.preferred_x, Some(column));
                Some(VerticalResult {
                    caret: Some(end),
                    preferred_x: column,
                })
            })
            .unwrap();
        assert_eq!(on_last_row.preferred_x, Some(column));

        let mut geometry_requested = false;
        let boundary = selection
            .move_endpoint(down, &document, |_| {
                geometry_requested = true;
                None
            })
            .unwrap();
        assert_eq!(boundary.caret, end);
        assert_eq!(boundary.preferred_x, None);
        assert!(!geometry_requested);
        selection
            .move_endpoint(up, &document, |motion| {
                assert_eq!(motion.previous, end);
                assert_eq!(motion.preferred_x, None);
                None
            })
            .unwrap();
    }

    #[test]
    fn a_foreign_pointer_target_cannot_start_a_drag() {
        let log = line_table_tests::source_log("2026-01-01 14:02:11 hello\n");
        let rows = LineTableRows::of(&log);
        let foreign = LineTableRows::of(&log);
        let mut selection = TableTextSelection::default();
        selection.replace_document(Some(rows.document_revision()));
        let limits = GestureLimits {
            distance: 6.0,
            duration: 0.6,
            multi_click_delay: 0.3,
        };
        let position = Pos2::ZERO;
        let caret = foreign.full_range().unwrap().anchor();

        selection.apply_pointer(
            PointerIntent::Press {
                position,
                time: 0.0,
                anchor: AnchorMode::Replace,
            },
            limits,
            |_, _| {
                Some(SelectionTarget {
                    caret,
                    range: CaretRange::point(caret),
                })
            },
        );
        assert!(selection.range().is_none());
        assert!(!selection.dragging());
        let mut target_requested = false;
        assert_eq!(
            selection.apply_pointer(
                PointerIntent::Release {
                    position,
                    time: 0.1
                },
                limits,
                |_, _| {
                    target_requested = true;
                    None
                }
            ),
            None,
        );
        assert!(!target_requested);
    }

    #[test]
    fn row_boundary_motion_rejects_a_caret_from_another_document() {
        let log = line_table_tests::source_log("2026-01-01 14:02:11 hello\n");
        let document = LineTableRows::of(&log);
        let foreign = LineTableRows::of(&log);
        let endpoint = NavigationEndpoint {
            caret: foreign.full_range().unwrap().endpoint(),
            preferred_x: None,
        };
        let motion = SelectionMotion::RowBoundary(Direction::Backward);
        assert!(endpoint.moved(motion, &document, |_| None).is_none());
    }
}
