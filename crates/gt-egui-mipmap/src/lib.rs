//! Min/max mipmap cascades for `egui_plot` time-series data.
//!
//! Both cascade types store [`egui_plot::PlotPoint`] levels in contiguous
//! buffers so callers can pass [`egui_plot::PlotPoints::Borrowed`] directly
//! to a plot line with no per-frame allocation.
//!
//! - [`MipMap`] stores `[x, y]` points and clips by `x` through
//!   [`SelectionRange`]. Use it for continuous plot-coordinate series such as
//!   context lines and snap error.
//! - [`KeyedMipMap`] builds from [`KeyedSample`] values, stores an ascending
//!   clipping key `K` and optional metadata `M` alongside each point, and
//!   clips against `K` through [`KeyBounds`]. Use it for recorded fix, metric,
//!   and channel series that clip against exact timestamps or attach
//!   per-sample metadata.
//!
//! Each coarser level groups `DOWNSAMPLE_WINDOW` points and emits the minimum
//! and maximum by `y` in input order. Passing a [`WrapPeriod`] selects the two
//! points furthest to either side of the window's circular mean direction when
//! every sample in the window lies in `[0, period)`.
//!
//! ## Complexity
//!
//! | Operation | Time | Space |
//! |---|---|---|
//! | `build` / `build_wrapping` / `build_with_period` | O(n) | O(n) |
//! | `select_indices` / `select_slice` | O(log n) | O(1) |
//! | `slice_at` / `key_at` / `meta_at` | O(1) | O(1) |

mod downsample;
mod keyed;
mod mipmap;
mod selection;
#[cfg(test)]
mod tests;
mod wrap;

pub use self::keyed::{KeyedMipMap, KeyedSample};
pub use self::mipmap::MipMap;
pub use self::selection::{KeyBounds, LevelSelection, SelectionRange};
pub use self::wrap::WrapPeriod;
