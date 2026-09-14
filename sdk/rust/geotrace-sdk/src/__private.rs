//! Internal support code for the `#[derive(EventKind)]` macro.
//!
//! Not part of the public API. Subject to change without notice.

/// The supertrait of [`EventKind`](crate::EventKind), which `#[derive(EventKind)]` implements.
///
/// A crate can implement it by hand as well: the derive expands in that crate, which can name every
/// path the derive emits.
pub trait Sealed {}
