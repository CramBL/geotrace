use gt_egui_mipmap::MipMap;
use gt_types::LoadedFile;
use gt_types::satellites::Constellation;
use uom::si::angle::degree;

/// Mipmap series for a single track.
///
/// Built from all points in the track regardless of current visibility or filter.
/// Visibility and time-range clamping are applied at render time in
/// [`super::plot_widget`] so the cache stays valid across filter changes.
#[derive(Debug, Clone)]
pub(crate) struct TrackSeries {
    /// File index within the loaded files list.
    pub fi: usize,
    /// Track index within that file.
    pub ti: usize,
    pub label: String,
    /// Precomputed `(x_min, x_max)` in Unix seconds, or `None` when the track
    /// has no points.  Computed once at build time from the first and last
    /// point timestamps - O(1) field access vs the previous `find_map` over
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
    /// GPS-clock lead over the host system clock, in milliseconds.
    /// Positive = GPS clock ahead, negative = system clock ahead.
    /// Only present when the TPV record carries a system timestamp.
    pub clock_delta_ms: MipMap,
}

/// Build mipmap series for every track in a single file, using `fi` as the file
/// index (for cache keying).
///
/// No visibility check or time filter is applied - that is done at render time
/// so the cache stays valid across filter changes without a rebuild.
pub(crate) fn build_file_series(fi: usize, file: &LoadedFile) -> Vec<TrackSeries> {
    file.tracks
        .iter()
        .enumerate()
        .map(|(ti, track)| build_track_series(fi, ti, file, track))
        .collect()
}

/// Build mipmap series for every track in every file.
pub(crate) fn build_all_series(files: &[LoadedFile]) -> Vec<TrackSeries> {
    files
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            file.tracks
                .iter()
                .enumerate()
                .map(move |(ti, track)| build_track_series(fi, ti, file, track))
        })
        .collect()
}

fn build_track_series(
    fi: usize,
    ti: usize,
    file: &LoadedFile,
    track: &gt_types::LoadedTrack,
) -> TrackSeries {
    let label = if file.tracks.len() == 1 {
        file.metadata.filename.clone()
    } else {
        format!("{} T{}", file.metadata.filename, ti + 1)
    };

    let mut total_seen_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut total_fix_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut gps_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut gps_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut glonass_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut glonass_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut galileo_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut galileo_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut beidou_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut beidou_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut velocity_kmh_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut eph_m_pts: Vec<[f64; 2]> = Vec::new();
    let mut heading_deg_pts: Vec<[f64; 2]> = Vec::new();
    let mut clock_delta_ms_pts: Vec<[f64; 2]> = Vec::new();

    for point in &track.points {
        let t = point.tpv.time().as_secs_f64();

        if let Some(sats) = &point.satellites {
            total_seen_pts.push([t, sats.satellite_count() as f64]);
            total_fix_pts.push([t, sats.fix_count() as f64]);

            let seen_and_fix = |c| {
                sats.by_constellation(c)
                    .fold((0usize, 0usize), |(seen, fix), sat| {
                        (seen + 1, fix + sat.in_fix() as usize)
                    })
            };
            let (gps_seen, gps_fix) = seen_and_fix(Constellation::Gps);
            let (gln_seen, gln_fix) = seen_and_fix(Constellation::Glonass);
            let (gal_seen, gal_fix) = seen_and_fix(Constellation::Galileo);
            let (bei_seen, bei_fix) = seen_and_fix(Constellation::Beidou);

            gps_seen_pts.push([t, gps_seen as f64]);
            gps_fix_pts.push([t, gps_fix as f64]);
            glonass_seen_pts.push([t, gln_seen as f64]);
            glonass_fix_pts.push([t, gln_fix as f64]);
            galileo_seen_pts.push([t, gal_seen as f64]);
            galileo_fix_pts.push([t, gal_fix as f64]);
            beidou_seen_pts.push([t, bei_seen as f64]);
            beidou_fix_pts.push([t, bei_fix as f64]);
        }

        if let Some(v) = point.tpv.velocity_kmh() {
            velocity_kmh_pts.push([t, v]);
        }

        if let Some(eph) = point.tpv.eph_m() {
            eph_m_pts.push([t, eph as f64]);
        }

        if let Some(h) = point.tpv.heading() {
            heading_deg_pts.push([t, h.get::<degree>()]);
        }

        if let Some(sys) = point.tpv.sys_time() {
            let delta_ms = point.tpv.time().offset_from_sys(sys).num_milliseconds();
            clock_delta_ms_pts.push([t, delta_ms as f64]);
        }
    }

    let x_range = track
        .points
        .first()
        .zip(track.points.last())
        .map(|(first, last)| {
            (
                first.tpv.time().as_secs_f64(),
                last.tpv.time().as_secs_f64(),
            )
        });

    TrackSeries {
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
        clock_delta_ms: MipMap::build(clock_delta_ms_pts),
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
            let da = (a.tpv.time().as_secs_f64() - target_secs).abs();
            let db = (b.tpv.time().as_secs_f64() - target_secs).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}
