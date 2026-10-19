use gt_ui_types::{PreparedMapEligibility, PreparedTrackEligibility, PreparedTrackPresence, PreparedTrackScope};

fn value<T>() -> T { loop {} }

fn main() {
    let _ = PreparedMapEligibility { eligibility: value(), tracks: value(), offsets: value() };
    let _ = PreparedTrackScope {
        track_ref: value(),
        track: None,
        visibility: None,
    };
    let _ = PreparedTrackEligibility { eligibility: value(), scope: value(), hidden_ranges: value() };
    let _ = PreparedTrackPresence { eligibility: value(), display_mask: value() };
}
