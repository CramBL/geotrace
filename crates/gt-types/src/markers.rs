use crate::coordinates::{Latitude, Longitude};
use crate::mercator::MercPoint;
use crate::satellites::SlipEvent;
use chrono::{DateTime, Duration, Utc};

/// RGB fill color for an event marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl MarkerColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

const EVENT_FALLBACK_COLORS: [MarkerColor; 8] = [
    MarkerColor::new(230, 57, 70),
    MarkerColor::new(255, 149, 0),
    MarkerColor::new(255, 190, 11),
    MarkerColor::new(6, 214, 160),
    MarkerColor::new(46, 196, 182),
    MarkerColor::new(131, 56, 236),
    MarkerColor::new(255, 45, 85),
    MarkerColor::new(238, 66, 102),
];

/// Deterministic fallback color for an unstyled event marker variant.
pub fn event_marker_fallback_color(variant_path: &str) -> MarkerColor {
    let mut hash: u64 = 5381;
    for b in variant_path.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(b));
    }
    #[expect(
        clippy::indexing_slicing,
        reason = "index is computed via modulo so always in bounds"
    )]
    EVENT_FALLBACK_COLORS[hash as usize % EVENT_FALLBACK_COLORS.len()]
}

/// An automatically-detected GNSS event, with the per-event payload carried in
/// the variant that needs it.
///
/// Not `Copy`/`Eq`: the [`Self::Slip`] payload owns a `Vec` of slipped
/// satellites (with `f32` SNR/elevation/azimuth).
#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedMarkerKind {
    GnssFixLost,
    GnssFixRegained {
        /// How long the fix was lost before being regained.
        fix_lost_duration: Duration,
    },
    /// The GPS−system clock offset jumped abruptly at this sample relative to
    /// the previous one, e.g. a device resuming from suspend, where a stale
    /// pre-suspend GPS timestamp meets a post-wake system timestamp.
    ClockDiscontinuity {
        /// Signed change in the GPS−system offset from the previous sample (the
        /// size of the jump).
        step: Duration,
    },
    /// The GPS−system clock offset left the track's baseline and returned.
    /// A whole recording gap lands in one sample's offset when the receiver
    /// reports its pre-gap GPS epoch for the first fix after the gap.  A device
    /// that holds its clock's boot default puts the offset between the two
    /// epochs on every fix until the receiver corrects the clock.
    /// [`Self::ClockDiscontinuity`] covers the other shape, an offset that
    /// steps and stays.
    ClockOffsetExcursion {
        /// Signed departure from the track's baseline offset at the furthest
        /// sample of the excursion.
        deviation: Duration,
        /// GPS−system offset at that same sample.
        offset: Duration,
        /// How many consecutive samples were out of band.
        samples: u32,
    },
    /// The receiver lost lock on one or more satellites that should still have
    /// been trackable - each vanished while above the elevation mask, or its SNR
    /// fell sharply between epochs.  Detected by `gt_analysis::loss_of_lock`; the
    /// payload groups every satellite that slipped at this epoch, each with its
    /// before/after geometry and signal so the marker can show what changed.
    Slip(SlipEvent),
}

impl std::fmt::Display for GeneratedMarkerKind {
    /// Canonical human-readable label.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::GnssFixLost => "GNSS fix lost",
            Self::GnssFixRegained { .. } => "GNSS fix regained",
            Self::ClockDiscontinuity { .. } => "Clock discontinuity",
            Self::ClockOffsetExcursion { .. } => "Clock offset excursion",
            Self::Slip(_) => "Satellite slip",
        })
    }
}

/// The kind of a [`GeneratedMarker`] with its payload stripped - a hashable,
/// orderable key for grouping markers by type and toggling per-type visibility.
///
/// The variant order is the canonical display order for the side-panel tree.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, strum::EnumIter, strum::EnumCount,
)]
pub enum GeneratedMarkerKindTag {
    GnssFixLost,
    GnssFixRegained,
    ClockDiscontinuity,
    ClockOffsetExcursion,
    Slip,
}

impl GeneratedMarkerKindTag {
    /// Human-readable label.  Kept identical to the matching
    /// [`GeneratedMarkerKind`] `Display` wording (asserted in tests).
    pub fn label(self) -> &'static str {
        match self {
            Self::GnssFixLost => "GNSS fix lost",
            Self::GnssFixRegained => "GNSS fix regained",
            Self::ClockDiscontinuity => "Clock discontinuity",
            Self::ClockOffsetExcursion => "Clock offset excursion",
            Self::Slip => "Satellite slip",
        }
    }
}

crate::enum_bitset! {
    /// A set of [`GeneratedMarkerKindTag`]s, one bit each, e.g. the per-track
    /// hidden tags in the generated-marker visibility state.
    pub struct GeneratedMarkerKindSet(u8) for GeneratedMarkerKindTag;
}

impl GeneratedMarkerKind {
    /// The payload-independent [`GeneratedMarkerKindTag`] for this marker.
    pub fn tag(&self) -> GeneratedMarkerKindTag {
        match self {
            Self::GnssFixLost => GeneratedMarkerKindTag::GnssFixLost,
            Self::GnssFixRegained { .. } => GeneratedMarkerKindTag::GnssFixRegained,
            Self::ClockDiscontinuity { .. } => GeneratedMarkerKindTag::ClockDiscontinuity,
            Self::ClockOffsetExcursion { .. } => GeneratedMarkerKindTag::ClockOffsetExcursion,
            Self::Slip(_) => GeneratedMarkerKindTag::Slip,
        }
    }
}

// Not `Copy`: `GeneratedMarkerKind::Slip` owns a `Vec` of slipped satellites.
#[derive(Debug, Clone)]
pub struct GeneratedMarker {
    pub time: DateTime<Utc>,
    pub kind: GeneratedMarkerKind,
    pub lat: Latitude,
    pub lon: Longitude,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

#[cfg(test)]
mod generated_marker_kind_tests {
    use super::*;

    /// Pins the canonical marker wording.
    #[test]
    fn label_is_canonical_wording() {
        assert_eq!(
            GeneratedMarkerKind::GnssFixLost.to_string(),
            "GNSS fix lost"
        );
        assert_eq!(
            GeneratedMarkerKind::GnssFixRegained {
                fix_lost_duration: Duration::zero()
            }
            .to_string(),
            "GNSS fix regained"
        );
        assert_eq!(
            GeneratedMarkerKind::ClockDiscontinuity {
                step: Duration::zero()
            }
            .to_string(),
            "Clock discontinuity"
        );
        assert_eq!(
            GeneratedMarkerKind::ClockOffsetExcursion {
                deviation: Duration::zero(),
                offset: Duration::zero(),
                samples: 1,
            }
            .to_string(),
            "Clock offset excursion"
        );
        assert_eq!(
            GeneratedMarkerKind::Slip(crate::satellites::SlipEvent {
                slips: vec![crate::satellites::Slip {
                    constellation: crate::satellites::Constellation::Gps,
                    prn: crate::satellites::Prn::new(1),
                    cause: crate::satellites::SlipCause::LostLock,
                    from: crate::satellites::SatSample {
                        elevation: None,
                        azimuth: None,
                        snr: None,
                    },
                    to: None,
                }],
            })
            .to_string(),
            "Satellite slip"
        );
    }

    /// Each kind's `tag().label()` must match its own `Display`, so the side
    /// panel's per-type headings never drift from the marker wording.
    #[test]
    fn tag_label_matches_display() {
        use strum::IntoEnumIterator as _;

        let sample = |tag: GeneratedMarkerKindTag| -> GeneratedMarkerKind {
            match tag {
                GeneratedMarkerKindTag::GnssFixLost => GeneratedMarkerKind::GnssFixLost,
                GeneratedMarkerKindTag::GnssFixRegained => GeneratedMarkerKind::GnssFixRegained {
                    fix_lost_duration: Duration::zero(),
                },
                GeneratedMarkerKindTag::ClockDiscontinuity => {
                    GeneratedMarkerKind::ClockDiscontinuity {
                        step: Duration::zero(),
                    }
                }
                GeneratedMarkerKindTag::ClockOffsetExcursion => {
                    GeneratedMarkerKind::ClockOffsetExcursion {
                        deviation: Duration::zero(),
                        offset: Duration::zero(),
                        samples: 1,
                    }
                }
                GeneratedMarkerKindTag::Slip => {
                    GeneratedMarkerKind::Slip(crate::satellites::SlipEvent { slips: vec![] })
                }
            }
        };
        for tag in GeneratedMarkerKindTag::iter() {
            let kind = sample(tag);
            assert_eq!(kind.tag(), tag, "tag() round-trips for {tag:?}");
            assert_eq!(
                tag.label(),
                kind.to_string(),
                "label matches Display for {tag:?}"
            );
        }
    }

    #[test]
    fn kind_set_membership_is_per_tag() {
        use strum::IntoEnumIterator as _;

        let mut set = GeneratedMarkerKindSet::empty();
        assert!(set.is_empty());
        assert!(GeneratedMarkerKindTag::iter().all(|t| !set.contains(t)));

        set.insert(GeneratedMarkerKindTag::Slip);
        assert!(set.contains(GeneratedMarkerKindTag::Slip));
        assert!(!set.is_empty());
        // Inserting one tag leaves every other tag out.
        assert!(
            GeneratedMarkerKindTag::iter()
                .filter(|&t| t != GeneratedMarkerKindTag::Slip)
                .all(|t| !set.contains(t))
        );
    }

    #[test]
    fn kind_set_from_iter_collects_each_tag_once() {
        use strum::IntoEnumIterator as _;

        let all: GeneratedMarkerKindSet = GeneratedMarkerKindTag::iter().collect();
        assert!(GeneratedMarkerKindTag::iter().all(|t| all.contains(t)));

        // Each tag occupies its own bit: a singleton contains only itself.
        for a in GeneratedMarkerKindTag::iter() {
            let only_a = GeneratedMarkerKindSet::single(a);
            assert!(GeneratedMarkerKindTag::iter().all(|b| only_a.contains(b) == (a == b)));
        }
    }
}

impl GeneratedMarker {
    pub fn new(
        time: DateTime<Utc>,
        kind: GeneratedMarkerKind,
        lat: Latitude,
        lon: Longitude,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            kind,
            lat,
            lon,
            merc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum MarkerIcon {
    Pin,
    Cross,
    Circle,
    Lightning,
    Warning,
    Error,
    Check,
    Satellite,
    SatelliteLost,
    Gear,
    Refresh,
    Download,
    Upload,
    Wrench,
}

#[derive(Debug, Clone)]
pub struct CustomMarker {
    pub time: DateTime<Utc>,
    pub label: String,
    pub icon: MarkerIcon,
    pub lat: Latitude,
    pub lon: Longitude,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

impl CustomMarker {
    pub fn new(
        time: DateTime<Utc>,
        label: String,
        icon: MarkerIcon,
        lat: Latitude,
        lon: Longitude,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            label,
            icon,
            lat,
            lon,
            merc,
        }
    }
}

/// File-level icon and color override for one event marker variant path.
#[derive(Debug, Clone)]
pub struct EventMarkerStyle {
    pub variant_path: String,
    /// Icon shape for this variant.
    pub icon: MarkerIcon,
    /// Fill color.
    pub color: MarkerColor,
}

/// A single event marker instance placed on the map.
#[derive(Debug, Clone)]
pub struct EventMarker {
    pub time: DateTime<Utc>,
    pub variant_path: String,
    pub annotation: Option<String>,
    pub lat: Latitude,
    pub lon: Longitude,
    /// Pre-computed normalized Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

impl EventMarker {
    pub fn new(
        time: DateTime<Utc>,
        variant_path: String,
        annotation: Option<String>,
        lat: Latitude,
        lon: Longitude,
    ) -> Self {
        let merc = crate::mercator::normalize(lat, lon);
        Self {
            time,
            variant_path,
            annotation,
            lat,
            lon,
            merc,
        }
    }
}
