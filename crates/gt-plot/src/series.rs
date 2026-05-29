use gt_egui_mipmap::MipMap;
use gt_types::LoadedFile;
use gt_types::satellites::Constellation;
use uom::si::angle::degree;
use uom::si::velocity::kilometer_per_hour;

/// Mipmap series for a single trip.
///
/// Built from all points in the trip regardless of current visibility or filter.
/// Visibility and time-range clamping are applied at render time in
/// [`super::plot_widget`] so the cache stays valid across filter changes.
#[derive(Debug, Clone)]
pub(crate) struct TripSeries {
    /// File index within the loaded files list.
    pub fi: usize,
    /// Trip index within that file.
    pub ti: usize,
    pub label: String,
    /// Precomputed `(x_min, x_max)` in Unix seconds, or `None` when the trip
    /// has no points.  Computed once at build time from the first and last
    /// point timestamps — O(1) field access vs the previous `find_map` over
    /// eight mipmaps.
    pub x_range: Option<(f64, f64)>,
    pub total_seen: MipMap,
    pub total_fix: MipMap,
    pub gps_seen: MipMap,
    pub gps_fix: MipMap,
    pub glonass_seen: MipMap,
    pub glonass_fix: MipMap,
    pub galileo_seen: MipMap,
    pub galileo_fix: MipMap,
    pub beidou_seen: MipMap,
    pub beidou_fix: MipMap,
    pub velocity_kmh: MipMap,
    pub eph_m: MipMap,
    pub heading_deg: MipMap,
}

/// Build mipmap series for every trip in a single file, using `fi` as the file
/// index (for cache keying).
///
/// No visibility check or time filter is applied — that is done at render time
/// so the cache stays valid across filter changes without a rebuild.
pub(crate) fn build_file_series(fi: usize, file: &LoadedFile) -> Vec<TripSeries> {
    let mut result = Vec::new();
    for (ti, trip) in file.trips.iter().enumerate() {
        result.push(build_trip_series(fi, ti, file, trip));
    }
    result
}

/// Build mipmap series for every trip in every file.
pub(crate) fn build_all_series(files: &[LoadedFile]) -> Vec<TripSeries> {
    let mut result = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        for (ti, trip) in file.trips.iter().enumerate() {
            result.push(build_trip_series(fi, ti, file, trip));
        }
    }
    result
}

fn build_trip_series(
    fi: usize,
    ti: usize,
    file: &LoadedFile,
    trip: &gt_types::LoadedTrip,
) -> TripSeries {
    let label = if file.trips.len() == 1 {
        file.metadata.filename.clone()
    } else {
        format!("{} T{}", file.metadata.filename, ti + 1)
    };

    let mut total_seen_pts: Vec<[f64; 2]> = Vec::with_capacity(trip.points.len());
    let mut total_fix_pts: Vec<[f64; 2]> = Vec::with_capacity(trip.points.len());
    let mut gps_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut gps_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut glonass_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut glonass_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut galileo_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut galileo_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut beidou_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut beidou_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut velocity_kmh_pts: Vec<[f64; 2]> = Vec::with_capacity(trip.points.len());
    let mut eph_m_pts: Vec<[f64; 2]> = Vec::new();
    let mut heading_deg_pts: Vec<[f64; 2]> = Vec::new();

    for point in &trip.points {
        let t = point.tpv.time().utc().timestamp() as f64;

        if let Some(sats) = &point.satellites {
            total_seen_pts.push([t, sats.satellite_count() as f64]);
            total_fix_pts.push([t, sats.fix_count() as f64]);

            let gps_s = sats.by_constellation(Constellation::Gps).count();
            let gps_f = sats
                .by_constellation(Constellation::Gps)
                .filter(|s| s.in_fix())
                .count();
            let gln_s = sats.by_constellation(Constellation::Glonass).count();
            let gln_f = sats
                .by_constellation(Constellation::Glonass)
                .filter(|s| s.in_fix())
                .count();
            let gal_s = sats.by_constellation(Constellation::Galileo).count();
            let gal_f = sats
                .by_constellation(Constellation::Galileo)
                .filter(|s| s.in_fix())
                .count();
            let bei_s = sats.by_constellation(Constellation::Beidou).count();
            let bei_f = sats
                .by_constellation(Constellation::Beidou)
                .filter(|s| s.in_fix())
                .count();

            gps_seen_pts.push([t, gps_s as f64]);
            gps_fix_pts.push([t, gps_f as f64]);
            glonass_seen_pts.push([t, gln_s as f64]);
            glonass_fix_pts.push([t, gln_f as f64]);
            galileo_seen_pts.push([t, gal_s as f64]);
            galileo_fix_pts.push([t, gal_f as f64]);
            beidou_seen_pts.push([t, bei_s as f64]);
            beidou_fix_pts.push([t, bei_f as f64]);
        }

        if let Some(vel) = point.tpv.velocity() {
            velocity_kmh_pts.push([t, vel.get::<kilometer_per_hour>()]);
        }

        if let Some(eph) = point.tpv.eph_m() {
            eph_m_pts.push([t, eph as f64]);
        }

        if let Some(h) = point.tpv.heading() {
            heading_deg_pts.push([t, h.get::<degree>()]);
        }
    }

    let x_range = trip
        .points
        .first()
        .zip(trip.points.last())
        .map(|(first, last)| {
            (
                first.tpv.time().utc().timestamp() as f64,
                last.tpv.time().utc().timestamp() as f64,
            )
        });

    TripSeries {
        fi,
        ti,
        label,
        x_range,
        total_seen: MipMap::build(total_seen_pts),
        total_fix: MipMap::build(total_fix_pts),
        gps_seen: MipMap::build(gps_seen_pts),
        gps_fix: MipMap::build(gps_fix_pts),
        glonass_seen: MipMap::build(glonass_seen_pts),
        glonass_fix: MipMap::build(glonass_fix_pts),
        galileo_seen: MipMap::build(galileo_seen_pts),
        galileo_fix: MipMap::build(galileo_fix_pts),
        beidou_seen: MipMap::build(beidou_seen_pts),
        beidou_fix: MipMap::build(beidou_fix_pts),
        velocity_kmh: MipMap::build(velocity_kmh_pts),
        eph_m: MipMap::build(eph_m_pts),
        heading_deg: MipMap::build(heading_deg_pts),
    }
}

/// Find the index of the point in `points` whose GPS timestamp is closest to
/// `target_secs` (Unix seconds).  Returns `None` if `points` is empty.
pub(crate) fn closest_point_index(
    points: &[gt_types::NavPoint],
    target_secs: f64,
) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = (a.tpv.time().utc().timestamp() as f64 - target_secs).abs();
            let db = (b.tpv.time().utc().timestamp() as f64 - target_secs).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}
