//! Assertions for the invariant every [`egui::Window`] in the app has to hold:
//! oversized content scrolls inside the window, which keeps the window inside
//! the screen edge. egui clips a window past that edge and puts its content out
//! of reach.

use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable as _};

/// Small enough in both axes that any window holding a few screenfuls of
/// content overflows it.
pub const CRAMPED_VIEWPORT: egui::Vec2 = egui::vec2(520.0, 380.0);

/// Narrow but tall: catches content that only overflows sideways, such as an
/// unbroken identity or log line in a horizontal layout.
pub const NARROW_VIEWPORT: egui::Vec2 = egui::vec2(360.0, 900.0);

/// Wide but short: catches content that only overflows downwards, such as a
/// long list above a footer.
pub const SHORT_VIEWPORT: egui::Vec2 = egui::vec2(1200.0, 300.0);

/// Characters [`oversized_text`] runs to: more than the widest audit viewport
/// fits, with no break for a wrap to take.
pub const OVERSIZED_TEXT_LENGTH: usize = 2000;

/// Rows an audit fixture lists: more than the tallest audit viewport shows at
/// once.
pub const OVERSIZED_ROW_COUNT: usize = 200;

/// An unbroken run of `fill`, standing in for the identity, path, URL or error
/// a window is handed. `fill` identifies the fixture it came from when it
/// turns up in a failure message.
pub fn oversized_text(fill: char) -> String {
    String::from(fill).repeat(OVERSIZED_TEXT_LENGTH)
}

/// Wheel points one scroll step sends, and how many steps
/// [`WindowFitAssertions::assert_control_is_reachable`] takes before it gives
/// up. Together they cover a list far longer than any window shows at once.
const WHEEL_POINTS_PER_STEP: f32 = 240.0;
const WHEEL_STEPS: usize = 60;

/// Frames a smooth scroll takes to come to rest.
const WHEEL_SETTLE_FRAMES: usize = 4;

/// Rounding slack when comparing a window rect against the viewport, in points.
const CONTAINMENT_SLACK: f32 = 1.0;

/// The [`egui::Window`] under audit: what to call it in a failure message, and
/// where egui keeps its rect.
#[derive(Clone, Copy)]
pub struct AuditedWindow<'a> {
    name: &'a str,
    area_id: egui::Id,
}

impl<'a> AuditedWindow<'a> {
    /// A window egui identifies by its title, which is what
    /// [`egui::Window::new`] derives the area id from.
    pub fn titled(title: &'a str) -> Self {
        Self {
            name: title,
            area_id: egui::Id::new(Some(title)),
        }
    }

    /// A window whose caller gave [`egui::Window::id`] an id of its own.
    pub fn identified(name: &'a str, area_id: egui::Id) -> Self {
        Self { name, area_id }
    }
}

/// The label of a control the user has to be able to reach - an action button,
/// a close affordance, the last row of a list.
#[derive(Clone, Copy)]
pub struct ControlLabel<'a>(pub &'a str);

pub trait WindowFitAssertions {
    /// Fails unless `window` lies inside the viewport.
    fn assert_window_fits_the_viewport(&self, window: AuditedWindow<'_>);

    /// Fails unless `control` is inside `window`, scrolling the window's
    /// contents towards it first.
    fn assert_control_is_reachable(&mut self, window: AuditedWindow<'_>, control: ControlLabel<'_>);
}

#[expect(
    clippy::panic,
    reason = "an assertion in a test harness reports its failure by panicking"
)]
impl<State> WindowFitAssertions for Harness<'_, State> {
    fn assert_window_fits_the_viewport(&self, window: AuditedWindow<'_>) {
        let viewport = self.ctx.content_rect();
        let rect = shown_window_rect(self, window);
        assert!(
            viewport.expand(CONTAINMENT_SLACK).contains_rect(rect),
            "the {:?} window is {:.0}x{:.0} at {rect:?}, outside the {:.0}x{:.0} viewport \
             {viewport:?}: its content needs to scroll instead of growing the window",
            window.name,
            rect.width(),
            rect.height(),
            viewport.width(),
            viewport.height(),
        );
    }

    fn assert_control_is_reachable(
        &mut self,
        window: AuditedWindow<'_>,
        control: ControlLabel<'_>,
    ) {
        let window_rect = shown_window_rect(self, window);
        let Some(mut rect) = control_rect(self, control) else {
            panic!(
                "no control labelled {:?} is rendered while the {:?} window is shown",
                control.0, window.name
            );
        };
        for _ in 0..WHEEL_STEPS {
            if window_rect.expand(CONTAINMENT_SLACK).contains_rect(rect) {
                return;
            }
            let towards = if rect.center().y < window_rect.center().y {
                WHEEL_POINTS_PER_STEP
            } else {
                -WHEEL_POINTS_PER_STEP
            };
            self.hover_at(window_rect.center());
            self.step();
            self.input_mut().events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, towards),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            self.run_steps(WHEEL_SETTLE_FRAMES);
            let Some(scrolled) = control_rect(self, control) else {
                panic!(
                    "the control labelled {:?} disappeared from the {:?} window while scrolling \
                     towards it",
                    control.0, window.name
                );
            };
            if scrolled == rect {
                break;
            }
            rect = scrolled;
        }
        panic!(
            "the control labelled {:?} sits at {rect:?}, outside the {:?} window at \
             {window_rect:?}, and no amount of scrolling brings it in: it is out of the user's \
             reach",
            control.0, window.name,
        );
    }
}

#[expect(
    clippy::panic,
    reason = "an assertion in a test harness reports its failure by panicking"
)]
fn shown_window_rect<State>(harness: &Harness<'_, State>, window: AuditedWindow<'_>) -> egui::Rect {
    match harness
        .ctx
        .memory(|memory| memory.area_rect(window.area_id))
    {
        Some(rect) => rect,
        None => panic!("the {:?} window is not shown", window.name),
    }
}

/// Where `control` is rendered, taking the lowest match so a label the toolbar
/// and the content share resolves to the content's.
fn control_rect<State>(
    harness: &Harness<'_, State>,
    control: ControlLabel<'_>,
) -> Option<egui::Rect> {
    harness
        .get_all(By::new().label(control.0))
        .map(|node| node.rect())
        .max_by(|a, b| a.top().total_cmp(&b.top()))
}
