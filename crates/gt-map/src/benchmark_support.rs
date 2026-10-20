//! Narrow entry points used by the Criterion baselines for map policy.
//!
//! Benchmarks need to exercise the production implementation without making
//! its internal plan types public.

use std::hint;

use gt_filter::GlobalFilter;
use gt_loaded_files::LoadedFilesView;
use gt_types::{GeoBounds, LoadedFile, SpatialPoint};
use gt_ui_types::{
    DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, MapEligibility, QueryMatches,
    SkyGlyphVariant, TrackDataVisibility,
};

use crate::MapFiles;
use crate::display_counts::{DisplayCounts, DisplayCountsCache, SuppliedCounts};
use crate::viewport::{self, MapFrameInputs, MapFramePlan};

/// Opaque handle to the production compiled frame plan used by benchmarks.
pub struct FramePlanBench<'a>(MapFramePlan<'a>);

impl FramePlanBench<'_> {
    /// The production candidate-local resolution path used by hover and marker renderers.
    pub fn spatial_point_visible(&self, point: &SpatialPoint) -> bool {
        self.0.resolve_spatial(point).is_some()
    }
}

/// Borrowed policy inputs used by the frame-plan benchmarks.
#[derive(Clone, Copy)]
pub struct FramePlanInputs<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    filter: &'a GlobalFilter,
    query_matches: Option<&'a QueryMatches>,
    generated_marker_visibility: &'a GeneratedMarkerVisibility,
    event_marker_visibility: &'a EventMarkerVisibility,
    display_mask: DisplayMask,
}

impl<'a> FramePlanInputs<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        filter: &'a GlobalFilter,
        query_matches: Option<&'a QueryMatches>,
        generated_marker_visibility: &'a GeneratedMarkerVisibility,
        event_marker_visibility: &'a EventMarkerVisibility,
        display_mask: DisplayMask,
    ) -> Self {
        Self {
            files,
            visibility,
            filter,
            query_matches,
            generated_marker_visibility,
            event_marker_visibility,
            display_mask,
        }
    }
}

/// Compile the production frame plan and keep its private representation behind
/// a narrow benchmark-only wrapper.
pub fn compile_frame_plan<'a>(inputs: FramePlanInputs<'a>, zoom: f64) -> FramePlanBench<'a> {
    FramePlanBench(hint::black_box(MapFramePlan::compute(
        MapFrameInputs::new(
            inputs.files,
            inputs.visibility,
            inputs.filter,
            inputs.query_matches,
            inputs.generated_marker_visibility,
            inputs.event_marker_visibility,
            inputs.display_mask,
        ),
        zoom,
    )))
}

#[derive(Default)]
pub struct CountsBench(DisplayCountsCache);

impl CountsBench {
    pub fn get(
        &mut self,
        files: LoadedFilesView<'_>,
        inputs: FramePlanInputs<'_>,
    ) -> DisplayCounts {
        self.0.get(
            MapFiles::Loaded(files),
            inputs.visibility,
            inputs.filter,
            inputs.event_marker_visibility,
            inputs.generated_marker_visibility,
            inputs.query_matches,
            SuppliedCounts::default(),
        )
    }
}

impl FramePlanInputs<'_> {
    pub fn count_elements(self) -> DisplayCounts {
        DisplayCounts::compute(
            self.files,
            self.visibility,
            self.filter,
            self.event_marker_visibility,
            self.generated_marker_visibility,
            self.query_matches,
            SuppliedCounts::default(),
        )
    }

    pub fn fit_bounds(self) -> Option<GeoBounds> {
        viewport::visible_bounds(
            MapEligibility::new(
                self.files,
                self.visibility,
                self.filter,
                self.query_matches,
                self.generated_marker_visibility,
                self.event_marker_visibility,
            )
            .with_display_mask(self.display_mask),
            SkyGlyphVariant::default(),
        )
    }
}
