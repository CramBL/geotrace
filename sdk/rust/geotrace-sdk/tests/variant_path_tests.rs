use geotrace_sdk::{Angle, DateTime, Duration, EventKind, NavFileBuilder, NavFix, Utc};

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn t(offset_secs: i64) -> DateTime<Utc> {
    base() + Duration::seconds(offset_secs)
}

fn fix() -> NavFix {
    NavFix::builder()
        .gps_time(t(0))
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .heading(Angle::degrees(0.0))
        .build()
}

// Unit variants produce the `snake_case` segment.
#[derive(EventKind)]
#[event_kind(note = none)]
enum FlatEvent {
    Boot,
    BatteryLow,
    GpsLockAcquired,
}

#[test]
fn unit_variant_produces_snake_case_segment() {
    assert_eq!(FlatEvent::Boot.variant_path().as_deref(), Some("boot"));
    assert_eq!(
        FlatEvent::BatteryLow.variant_path().as_deref(),
        Some("battery_low")
    );
    assert_eq!(
        FlatEvent::GpsLockAcquired.variant_path().as_deref(),
        Some("gps_lock_acquired")
    );
}

// Single-field tuple variants delegate in strict mode (the default).
#[derive(EventKind)]
#[event_kind(note = none)]
enum Inner {
    Request,
    Success,
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum Middle {
    Agps(Inner),
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum Outer {
    Connectivity(Middle),
}

#[test]
fn delegation_concatenates_three_levels() {
    let path = Outer::Connectivity(Middle::Agps(Inner::Request)).variant_path();
    assert_eq!(path.as_deref(), Some("connectivity/agps/request"));
}

// Lax mode: single-field tuple variants are leaves by default.
// Use #[event_kind(delegate)] to explicitly opt in to delegation.
struct NotEventKind;

#[derive(EventKind)]
#[event_kind(lax, note = none)]
enum LaxEvent {
    #[expect(dead_code, reason = "field only used as delegation target")]
    Typed(Inner),
    Raw(NotEventKind),
    #[event_kind(delegate)]
    Explicit(Inner),
}

#[test]
fn lax_default_is_leaf_even_when_inner_has_trait() {
    let path = LaxEvent::Typed(Inner::Success).variant_path();
    assert_eq!(path.as_deref(), Some("typed"));
}

#[test]
fn lax_leaf_when_inner_lacks_trait() {
    let path = LaxEvent::Raw(NotEventKind).variant_path();
    assert_eq!(path.as_deref(), Some("raw"));
}

#[test]
fn lax_delegate_attr_forces_delegation() {
    let path = LaxEvent::Explicit(Inner::Success).variant_path();
    assert_eq!(path.as_deref(), Some("explicit/success"));
}

// Multi-field tuple variant → leaf segment.
#[derive(EventKind)]
#[event_kind(note = none)]
enum MultiTupleEvent {
    #[expect(dead_code, reason = "fields only for derive shape test")]
    Location(f64, f64),
}

#[test]
fn multi_field_tuple_variant_is_leaf() {
    assert_eq!(
        MultiTupleEvent::Location(1.0, 2.0)
            .variant_path()
            .as_deref(),
        Some("location")
    );
}

// Struct variant → leaf segment.
#[derive(EventKind)]
#[event_kind(note = none)]
enum StructEvent {
    #[expect(dead_code, reason = "struct variant used only for derive test")]
    Packet { size: u32, flags: u8 },
}

#[test]
fn struct_variant_is_leaf() {
    assert_eq!(
        StructEvent::Packet { size: 42, flags: 0 }
            .variant_path()
            .as_deref(),
        Some("packet")
    );
}

// Skip attribute returns None.
#[derive(EventKind)]
#[event_kind(note = none)]
enum SkipEvent {
    Active,
    #[event_kind(skip)]
    Internal,
}

#[test]
fn skip_variant_returns_none() {
    assert_eq!(SkipEvent::Active.variant_path().as_deref(), Some("active"));
    assert!(SkipEvent::Internal.variant_path().is_none());
}

// `add_event` silently no-ops when the variant returns `None`.
#[test]
fn add_event_noop_on_skip_variant() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix());
    recorder.add_event(&SkipEvent::Internal, t(0));
    let nav_file = recorder.finish().unwrap();
    assert_eq!(nav_file.event_markers().len(), 0);
}

#[test]
fn add_event_adds_marker_on_non_skip_variant() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix());
    recorder.add_event(&SkipEvent::Active, t(0));
    let nav_file = recorder.finish().unwrap();
    assert_eq!(nav_file.event_markers().len(), 1);
    assert_eq!(nav_file.event_markers()[0].variant_path, "active");
}

// #[event_kind(leaf)] forces leaf even when inner type implements EventKind.
#[derive(EventKind)]
#[event_kind(note = none)]
enum LeafOverrideEvent {
    Delegate(Inner),
    #[event_kind(leaf)]
    #[expect(dead_code, reason = "field unused; leaf stops delegation")]
    ForceLeaf(Inner),
}

#[test]
fn leaf_attr_prevents_delegation() {
    assert_eq!(
        LeafOverrideEvent::Delegate(Inner::Request)
            .variant_path()
            .as_deref(),
        Some("delegate/request")
    );
    assert_eq!(
        LeafOverrideEvent::ForceLeaf(Inner::Request)
            .variant_path()
            .as_deref(),
        Some("force_leaf")
    );
}

// In strict mode, #[event_kind(lax)] on a variant is an alias for leaf.
#[derive(EventKind)]
#[event_kind(strict, note = none)]
enum StrictEvent {
    Typed(Inner),
    #[event_kind(lax)]
    Optional(NotEventKind),
    #[event_kind(leaf)]
    LeafOnly(NotEventKind),
}

#[test]
fn strict_delegates_typed_variant() {
    assert_eq!(
        StrictEvent::Typed(Inner::Request).variant_path().as_deref(),
        Some("typed/request")
    );
}

#[test]
fn lax_attr_in_strict_mode_acts_as_leaf() {
    assert_eq!(
        StrictEvent::Optional(NotEventKind)
            .variant_path()
            .as_deref(),
        Some("optional")
    );
}

#[test]
fn leaf_attr_in_strict_mode_forces_leaf() {
    assert_eq!(
        StrictEvent::LeafOnly(NotEventKind)
            .variant_path()
            .as_deref(),
        Some("leaf_only")
    );
}
