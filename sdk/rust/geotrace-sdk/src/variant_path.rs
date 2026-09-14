use crate::__private::Sealed;

/// Implemented by `enum` types to produce a slash-separated variant path string.
///
/// Use `#[derive(EventKind)]` to implement it.  Each variant's name is
/// converted to `snake_case` and becomes one segment of the path.  Nested
/// `enum` types produce paths like `"power/boot"` or
/// `"connectivity/agps/request"` by chaining segments.
///
/// A run of capitals is one word, apart from a last capital before a lower-case
/// letter: `GPS3Lock` gives `gps3_lock` and `HTTPError` gives `http_error`. A
/// raw identifier loses its `r#` (`r#type` gives `type`). A segment has only ASCII
/// letters, digits, `-` and `_`, and at most 255 bytes, and no two variants of
/// one `enum` have the same segment. The derive reports a compile error on a
/// variant that breaks either rule, and `#[event_kind(rename = "<segment>")]`
/// sets another segment for it. A nested path past 255 bytes makes
/// [`NavRecorder::finish`](crate::NavRecorder::finish) fail.
///
/// [`variant_path`](EventKind::variant_path) returns `None` for variants
/// marked `#[event_kind(skip)]`. Callers such as
/// [`NavRecorder::add_event`](crate::NavRecorder::add_event) treat `None` as a
/// silent no-op.
///
/// # Derive attributes
///
/// ## Enum-level
///
/// Place one of these on the `enum` itself to set the default behaviour for
/// single-field tuple variants.
///
/// | Attribute | Effect |
/// |-----------|--------|
/// | *(none)* / `#[event_kind(strict)]` | Single-field tuple variants **delegate** to the inner type by default (compile error if the inner type does not implement `EventKind`). This is the default. |
/// | `#[event_kind(lax)]` | Single-field tuple variants are **leaves** by default - they emit only their own segment and do not call into the inner type.  Use `#[event_kind(delegate)]` on specific variants to opt into delegation. |
///
/// ## Variant-level
///
/// Place one of these on an individual variant to override the enum-level
/// default for that variant alone.
///
/// | Attribute | Effect |
/// |-----------|--------|
/// | `#[event_kind(leaf)]` | Always emit only this variant's segment. Never delegate, even if the inner type implements `EventKind`. |
/// | `#[event_kind(delegate)]` | Always delegate to the inner `EventKind` implementation, appending its path after this variant's segment.  Required in `lax` mode when you *do* want delegation. Only a variant with one unnamed field delegates. |
/// | `#[event_kind(skip)]` | `variant_path()` returns `None` for this variant. `add_event` silently ignores it. |
/// | `#[event_kind(icon = <Name>)]` | Sets the [`MarkerIcon`](crate::MarkerIcon) for this variant.  `<Name>` must be a variant of `MarkerIcon` (e.g. `Warning`, `Check`).  A delegating variant takes its icon from the inner type's leaf. |
/// | `#[event_kind(rename = "<segment>")]` | Sets this variant's segment in place of the one derived from its name. |
///
/// The derive reports a compile error for an attribute without effect on its
/// variant, such as `icon` on a skipped or a delegating variant, and for a
/// second attribute of one kind, such as `leaf` after `delegate`, `lax` after
/// `strict` or a second `note`.
///
/// # Example
///
/// In strict mode (the default), wrapping a type that doesn't implement `EventKind`
/// is a compile error:
///
/// ```compile_fail
/// use geotrace_sdk::EventKind;
///
/// #[derive(EventKind)]
/// enum Event {
///     Power(PowerEvent),
///     Debug(String),  // compile error - String: !EventKind
/// }
///
/// #[derive(EventKind)]
/// enum PowerEvent { Boot }
/// ```
///
/// Use `#[event_kind(lax)]` when inner types don't implement `EventKind`:
///
/// ```rust
/// use geotrace_sdk::EventKind;
///
/// #[derive(Debug, EventKind)]
/// #[event_kind(lax)]
/// enum SafeEvent {
///     Power(PowerEvent),          // leaf by default in lax mode
///     #[event_kind(delegate)]
///     Sensor(SensorEvent),        // explicitly opt into delegation
///     #[event_kind(skip)]
///     Internal(String),           // variant_path() returns None
/// }
///
/// #[derive(Debug, EventKind)]
/// enum PowerEvent { Boot, Sleep }
///
/// #[derive(Debug, EventKind)]
/// enum SensorEvent { GpsLock }
///
/// assert_eq!(
///     SafeEvent::Sensor(SensorEvent::GpsLock).variant_path().as_deref(),
///     Some("sensor/gps_lock"),
/// );
/// assert_eq!(SafeEvent::Internal("x".into()).variant_path(), None);
/// ```
pub trait EventKind: Sealed {
    fn variant_path(&self) -> Option<String>;

    /// The icon declared on this variant via `#[event_kind(icon = <Name>)]`, if any.
    ///
    /// For delegating variants the inner type's `marker_icon` is returned, so the
    /// icon always comes from the leaf regardless of nesting depth.
    /// Returns `None` when no icon was specified. Callers fall back to the application default.
    fn marker_icon(&self) -> Option<crate::MarkerIcon> {
        None
    }

    /// A short human-readable note for this event instance, shown alongside the marker in the app.
    ///
    /// Controlled by the enum-level `#[event_kind(note = ...)]` attribute:
    ///
    /// | Attribute | Behaviour |
    /// |-----------|-----------|
    /// | *(none)* / `#[event_kind(note = debug)]` | `Some(format!("{self:?}"))` - requires `Debug`. This is the default. |
    /// | `#[event_kind(note = display)]` | `Some(format!("{self}"))` - requires `Display`. |
    /// | `#[event_kind(note = none)]` | Always `None`. No note is stored. |
    ///
    /// For a one-off custom note on a specific event instance use
    /// [`NavRecorder::add_event_with_note`](crate::NavRecorder::add_event_with_note)
    /// directly - it overrides `event_note` entirely.
    fn event_note(&self) -> Option<String> {
        None
    }
}
