use std::thread::sleep;
use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::kittest::{By, Queryable as _};
use egui_kittest::{Harness, Node};

/// Frames [`HarnessInteraction::step_until`] runs before giving up, and the
/// pause it leaves between them for background threads to make progress. Two
/// seconds in total, which covers a file load or a database write on a busy
/// machine.
const STEP_UNTIL_FRAME_BUDGET: usize = 200;
const PAUSE_BETWEEN_FRAMES: Duration = Duration::from_millis(10);

/// Queues `clicks` primary press-and-release pairs at `target`, all read by
/// the frame that runs next.
fn queue_primary_clicks<State>(harness: &mut Harness<'_, State>, target: egui::Pos2, clicks: u8) {
    for _ in 0..clicks {
        for pressed in [true, false] {
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
}

/// Driving an [`egui_kittest::Harness`]: waiting on background work, pointer
/// interaction, and picking one node out of several that share a label.
pub trait HarnessInteraction {
    /// Runs one frame at a time, pausing between them so background threads
    /// make progress, until `predicate` holds. Returns whether it held within
    /// the budget.
    #[must_use]
    fn step_until(&mut self, predicate: impl FnMut(&Self) -> bool) -> bool;

    /// [`HarnessInteraction::step_until`] for work that produces a value,
    /// returning the first value `read` yields.
    #[must_use]
    fn step_until_some<T>(&mut self, read: impl FnMut(&Self) -> Option<T>) -> Option<T>;

    /// Focuses the one text input on screen and types `text` into it, then
    /// runs the frames the edit needs to reach the state behind the field.
    fn type_into_text_input(&mut self, text: &str);

    /// [`HarnessInteraction::hover_at_and_settle`] at the centre of the node
    /// matching `by`.
    fn hover_and_settle(&mut self, by: By<'_>, settle_frames: usize);

    /// Moves the pointer to `target` and holds it there for `settle_frames`.
    ///
    /// egui opens a tooltip once the pointer has been still for `tooltip_delay`
    /// and every `PointerMoved` restarts that timer: the move is sent once and
    /// the settle frames then run without any event.
    fn hover_at_and_settle(&mut self, target: egui::Pos2, settle_frames: usize);

    fn press_drag_release(&mut self, from: egui::Pos2, delta: egui::Vec2, move_frames: u16);

    /// Presses and releases at `target` within one frame, which egui reads as
    /// a click.
    fn click_at(&mut self, target: egui::Pos2);

    /// [`HarnessInteraction::click_at`] without the pointer movement it makes
    /// first: this is the press of a user whose pointer already rests on the
    /// control they aimed at.
    fn press_where_the_pointer_rests(&mut self, target: egui::Pos2);

    /// Presses and releases twice at `target` within one frame, which egui
    /// reads as a double click.
    ///
    /// The harness gives every queued event a frame of its own, and its clock
    /// ticks a quarter second per frame: two clicks queued one after the other
    /// land further apart than egui's double-click window.
    fn double_click_at(&mut self, target: egui::Pos2);

    /// Moves the pointer to `target`, sends one wheel scroll of `delta_points`
    /// there (negative scrolls towards the end of the content), and runs the
    /// frames the smooth scroll takes to come to rest.
    fn scroll_wheel_at(&mut self, target: egui::Pos2, delta_points: f32, settle_frames: usize);

    /// The matching node with the smallest `rect().top()`, for labels that
    /// several widgets on screen share.
    fn topmost_matching<'t>(&'t self, by: By<'t>) -> Node<'t>;

    /// The matching node with the largest `rect().top()`.
    fn bottommost_matching<'t>(&'t self, by: By<'t>) -> Node<'t>;

    /// The matching node at `index`, in tree order.
    fn nth_matching<'t>(&'t self, by: By<'t>, index: usize) -> Node<'t>;

    /// Screen rect of the [`egui::Window`] titled `title`, `None` while it is
    /// not shown. [`egui::Window::new`] derives the area id from the title as
    /// `Id::new(Some(title))`.
    fn window_rect(&self, title: &str) -> Option<egui::Rect>;

    /// Runs `settle_frames` frames, then measures the window titled `title`. A
    /// resizable window sizes itself against its content over several frames.
    fn settled_window_size(&mut self, title: &str, settle_frames: usize) -> Option<egui::Vec2>;
}

impl<State> HarnessInteraction for Harness<'_, State> {
    fn step_until(&mut self, mut predicate: impl FnMut(&Self) -> bool) -> bool {
        self.step_until_some(|harness| predicate(harness).then_some(()))
            .is_some()
    }

    fn step_until_some<T>(&mut self, mut read: impl FnMut(&Self) -> Option<T>) -> Option<T> {
        for _ in 0..STEP_UNTIL_FRAME_BUDGET {
            if let Some(value) = read(self) {
                return Some(value);
            }
            sleep(PAUSE_BETWEEN_FRAMES);
            self.step();
        }
        read(self)
    }

    fn type_into_text_input(&mut self, text: &str) {
        let field = self.get_by(|node| node.role() == Role::TextInput);
        field.focus();
        field.type_text(text);
        self.run();
    }

    fn hover_and_settle(&mut self, by: By<'_>, settle_frames: usize) {
        let target = self.get(by).rect().center();
        self.hover_at_and_settle(target, settle_frames);
    }

    fn hover_at_and_settle(&mut self, target: egui::Pos2, settle_frames: usize) {
        self.hover_at(target);
        self.run_steps(settle_frames);
    }

    fn press_drag_release(&mut self, from: egui::Pos2, delta: egui::Vec2, move_frames: u16) {
        self.hover_at(from);
        self.step();
        self.drag_at(from);
        self.step();
        for frame in 1..=move_frames {
            self.hover_at(from + delta * (f32::from(frame) / f32::from(move_frames)));
            self.step();
        }
        self.drop_at(from + delta);
        self.step();
    }

    fn click_at(&mut self, target: egui::Pos2) {
        self.hover_at(target);
        self.step();
        queue_primary_clicks(self, target, 1);
        self.step();
    }

    fn press_where_the_pointer_rests(&mut self, target: egui::Pos2) {
        queue_primary_clicks(self, target, 1);
        self.run_steps(2);
    }

    fn double_click_at(&mut self, target: egui::Pos2) {
        self.hover_at(target);
        self.step();
        queue_primary_clicks(self, target, 2);
        self.step();
    }

    fn scroll_wheel_at(&mut self, target: egui::Pos2, delta_points: f32, settle_frames: usize) {
        self.hover_at(target);
        self.step();
        self.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta_points),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        self.run_steps(settle_frames);
    }

    #[expect(
        clippy::expect_used,
        reason = "get_all already panics when nothing matches"
    )]
    fn topmost_matching<'t>(&'t self, by: By<'t>) -> Node<'t> {
        self.get_all(by)
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("a matching node")
    }

    #[expect(
        clippy::expect_used,
        reason = "get_all already panics when nothing matches"
    )]
    fn bottommost_matching<'t>(&'t self, by: By<'t>) -> Node<'t> {
        self.get_all(by)
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("a matching node")
    }

    #[expect(
        clippy::expect_used,
        reason = "an out-of-range index in a test is a fatal setup error"
    )]
    fn nth_matching<'t>(&'t self, by: By<'t>, index: usize) -> Node<'t> {
        self.get_all(by)
            .nth(index)
            .expect("a matching node at the index")
    }

    fn window_rect(&self, title: &str) -> Option<egui::Rect> {
        self.ctx
            .memory(|memory| memory.area_rect(egui::Id::new(Some(title))))
    }

    fn settled_window_size(&mut self, title: &str, settle_frames: usize) -> Option<egui::Vec2> {
        for _ in 0..settle_frames {
            self.run();
        }
        self.window_rect(title).map(|rect| rect.size())
    }
}
