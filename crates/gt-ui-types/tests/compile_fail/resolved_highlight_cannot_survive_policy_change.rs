use gt_filter::GlobalFilter;
use gt_ui_types::{
    DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, MapEligibility, MapHighlight,
    TrackDataVisibility,
};

fn main() {
    let mut filter = GlobalFilter::default();
    let visibility = TrackDataVisibility::from_loaded(&[]);
    let generated = GeneratedMarkerVisibility::default();
    let events = EventMarkerVisibility::default();
    let mut highlight = MapHighlight::default();
    let resolved = highlight.resolve(
        MapEligibility::new(&[], &visibility, &filter, None, &generated, &events)
            .with_display_mask(DisplayMask::default()),
    );
    filter.set_time_bounds(None, None);
    highlight.plot_hover_point = None;
    let _ = resolved.snapped_plot_hover_track();
}
