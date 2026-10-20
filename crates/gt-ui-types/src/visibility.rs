use std::ops::Range;

use chrono::{DateTime, Utc};
use gt_filter::GlobalFilter;
use gt_types::satellites::Satellites;
use gt_types::{
    CustomMarker, DataCategory, DataCategorySet, EventMarker, FileIdx, GeneratedMarker, LoadedFile,
    LoadedTrack, NavPoint, PointIdx, TrackIdx, TrackRef,
};
use smallvec::SmallVec;
use strum::EnumCount;

use crate::display_mask::{DisplayCategory, DisplayMask};
use crate::event_marker_visibility::EventMarkerVisibility;
use crate::generated_marker_visibility::GeneratedMarkerVisibility;
use crate::highlight::MapElementRef;
use crate::query_matches::QueryMatches;

#[derive(Debug, Clone, PartialEq)]
pub struct TrackDataVisibility {
    pub files: Vec<FileVisibility>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileVisibility {
    pub enabled: bool,
    pub tracks: Vec<TrackVisibility>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackVisibility {
    pub enabled: bool,
    /// Per-category tree toggles. `enabled` is separate: it gates the whole
    /// track, while this only hides individual element categories.
    categories: DataCategorySet,
}

impl TrackVisibility {
    /// This track's tree toggle for `category`, the single mapping renderers
    /// and counts consult.
    pub fn category_visible(self, category: DataCategory) -> bool {
        self.categories.contains(category)
    }

    /// Show or hide `category` for this track.
    pub fn set_category_visible(&mut self, category: DataCategory, visible: bool) {
        self.categories.set(category, visible);
    }

    pub fn all_visible() -> Self {
        Self {
            enabled: true,
            categories: DataCategorySet::all(),
        }
    }
}

impl TrackDataVisibility {
    pub fn from_loaded(files: &[LoadedFile]) -> Self {
        Self {
            files: files
                .iter()
                .map(|f| FileVisibility {
                    enabled: true,
                    tracks: f
                        .tracks
                        .iter()
                        .map(|_| TrackVisibility::all_visible())
                        .collect(),
                })
                .collect(),
        }
    }

    /// Enable or disable every file and track at once.
    pub fn set_all_enabled(&mut self, enabled: bool) {
        for file in &mut self.files {
            file.enabled = enabled;
            for track in &mut file.tracks {
                track.enabled = enabled;
            }
        }
    }

    /// Whether `track_ref` and its file are enabled.
    pub fn track_enabled(&self, track_ref: TrackRef) -> bool {
        track_ref
            .fi
            .get(&self.files)
            .is_some_and(|f| f.enabled && track_ref.index.get(&f.tracks).is_some_and(|t| t.enabled))
    }

    /// Whether `track_ref`'s line is shown on the map: its file and the track
    /// are enabled, and the track-line toggle is on. The predicate the
    /// snapped-track rendering and the snap queue's visibility priority use.
    pub fn track_shown(&self, track_ref: TrackRef) -> bool {
        track_ref.fi.get(&self.files).is_some_and(|f| {
            f.enabled
                && track_ref
                    .index
                    .get(&f.tracks)
                    .is_some_and(|t| t.enabled && t.category_visible(DataCategory::Track))
        })
    }

    /// Show only `fi`. Hide all others. Track visibility within files is
    /// preserved so that re-enabling a file restores its previous state.
    pub fn show_only_file(&mut self, fi: FileIdx) {
        for (i, file) in self.files.iter_mut().enumerate() {
            file.enabled = FileIdx::new(i) == fi;
        }
    }

    /// Show only `track` and its parent file. Hide everything else.
    pub fn show_only_track(&mut self, track: TrackRef) {
        for (i, file) in self.files.iter_mut().enumerate() {
            if FileIdx::new(i) == track.fi {
                file.enabled = true;
                for (j, t) in file.tracks.iter_mut().enumerate() {
                    t.enabled = TrackIdx::new(j) == track.index;
                }
            } else {
                file.enabled = false;
            }
        }
    }
}

/// The track-level gate: file and track enabled, track filter passed.
/// Returns the resolved track and its tree toggles when everything passes.
pub fn track_in_scope<'a>(
    files: &'a [LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    track_ref: TrackRef,
) -> Option<(&'a LoadedTrack, TrackVisibility)> {
    let prepared = PreparedTrackScope::new(files, visibility, filter, track_ref);
    Some((prepared.track?, prepared.visibility?))
}

/// Why the element a [`MapElementRef`] addresses is, or is not, on the map.
///
/// This is the compatibility view used by renderers and tests. Internally,
/// [`MapEligibility`] owns semantic policy and [`MapPresence`] adds only the
/// display mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount)]
pub enum PointVisibility {
    /// Drawn on the map.
    Shown,
    /// Nothing is addressed: the file or track is not loaded, or the typed
    /// index is past the end of its array.
    NoSuchElement,
    /// The file or track is off in the tree, or the track fails the track-level
    /// filter.
    TrackNotShown,
    /// The element's category is off, either in the track's tree toggles or in
    /// the display mask.
    CategoryHidden,
    /// The marker's type is off in the tree: the kind of a generated marker,
    /// or the variant path of an event marker or a parent path of it.
    MarkerTypeHidden,
    /// A `keep` or `hide` query removed the point.
    HiddenByQuery,
    /// Outside the global time filter's window.
    OutsideTimeFilter,
}

impl PointVisibility {
    pub fn is_shown(self) -> bool {
        self == Self::Shown
    }
}

/// Why an existing element is withheld before display masking is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EligibilityWithheld {
    /// The file or track is off in the tree, or the track fails the filter.
    TrackNotShown,
    /// The element's category is off in the track's tree toggles.
    CategoryHidden,
    /// The marker's generated kind or event path is hidden.
    MarkerTypeHidden,
    /// A `keep` or `hide` query removed the fix-backed element.
    HiddenByQuery,
    /// The element lies outside the global time window.
    OutsideTimeFilter,
}

/// The result of applying all semantic map policy except the display mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapEligibilityResult {
    /// The addressed element exists and passes every semantic gate.
    Eligible,
    /// The raw identity does not resolve against the currently loaded files.
    Missing,
    /// The element exists, but semantic policy withholds it.
    Withheld(EligibilityWithheld),
}

/// Every policy source that determines whether a point-like element is
/// semantically eligible for the map, before render-side display masking.
///
/// All sources are mandatory at construction so callers cannot accidentally
/// evaluate an element against only a subset of map policy.
#[derive(Clone, Copy)]
pub struct MapEligibility<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    filter: &'a GlobalFilter,
    query_matches: Option<&'a QueryMatches>,
    generated_marker_visibility: &'a GeneratedMarkerVisibility,
    event_marker_visibility: &'a EventMarkerVisibility,
}

impl<'a> MapEligibility<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        filter: &'a GlobalFilter,
        query_matches: Option<&'a QueryMatches>,
        generated_marker_visibility: &'a GeneratedMarkerVisibility,
        event_marker_visibility: &'a EventMarkerVisibility,
    ) -> Self {
        Self {
            files,
            visibility,
            filter,
            query_matches,
            generated_marker_visibility,
            event_marker_visibility,
        }
    }

    /// Add render-side display policy to this complete semantic policy.
    pub fn with_display_mask(self, display_mask: DisplayMask) -> MapPresence<'a> {
        MapPresence::new(self, display_mask)
    }

    /// Classify `element_ref` before display masking.
    #[inline]
    pub fn element_eligibility(self, element_ref: MapElementRef) -> MapEligibilityResult {
        match self.resolve(element_ref) {
            ResolvedEligibility::Eligible(_) => MapEligibilityResult::Eligible,
            ResolvedEligibility::Missing => MapEligibilityResult::Missing,
            ResolvedEligibility::Withheld(reason) => MapEligibilityResult::Withheld(reason),
        }
    }

    pub fn prepare(self) -> PreparedMapEligibility<'a> {
        self.prepare_with_track_observer(|_| {})
    }

    pub fn prepare_with_track_observer(
        self,
        mut observe_track_filter: impl FnMut(&LoadedTrack),
    ) -> PreparedMapEligibility<'a> {
        let mut tracks = SmallVec::new();
        let mut offsets = SmallVec::new();
        for (fi, file) in self.files.iter().enumerate() {
            offsets.push(tracks.len());
            for ti in 0..file.tracks.len() {
                let scope = PreparedTrackScope::with_track_observer(
                    self.files,
                    self.visibility,
                    self.filter,
                    TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)),
                    &mut observe_track_filter,
                );
                tracks.push(scope.visibility);
            }
        }
        offsets.push(tracks.len());
        PreparedMapEligibility {
            eligibility: self,
            tracks,
            offsets,
        }
    }

    pub fn query_matches(self) -> Option<&'a QueryMatches> {
        self.query_matches
    }

    pub fn prepare_track(self, track_ref: TrackRef) -> PreparedTrackEligibility<'a> {
        PreparedTrackEligibility::new(
            self,
            PreparedTrackScope::new(self.files, self.visibility, self.filter, track_ref),
        )
    }

    fn resolve(self, element_ref: MapElementRef) -> ResolvedEligibility<'a> {
        self.prepare_track(element_ref.track()).resolve(element_ref)
    }
}

pub struct PreparedMapEligibility<'a> {
    eligibility: MapEligibility<'a>,
    tracks: SmallVec<[Option<TrackVisibility>; 128]>,
    offsets: SmallVec<[usize; 9]>,
}

impl<'a> PreparedMapEligibility<'a> {
    #[inline]
    pub fn flat_track_index(&self, track: TrackRef) -> Option<usize> {
        self.track_slice().flat_track_index(track)
    }

    #[inline]
    pub fn track(&self, track: TrackRef) -> Option<PreparedTrackEligibility<'a>> {
        self.track_slice().track(track, self.eligibility)
    }

    pub fn with_display_mask(&self, display_mask: DisplayMask) -> MapPresence<'_> {
        MapPresence {
            eligibility: self.eligibility,
            display_mask,
            prepared: Some(self.track_slice()),
        }
    }

    pub fn uses_inline_storage(&self) -> bool {
        !self.tracks.spilled() && !self.offsets.spilled()
    }

    fn track_slice(&self) -> PreparedTrackSlice<'_> {
        PreparedTrackSlice {
            tracks: &self.tracks,
            offsets: &self.offsets,
        }
    }
}

#[derive(Clone, Copy)]
pub struct PreparedTrackScope<'a> {
    track_ref: TrackRef,
    track: Option<&'a LoadedTrack>,
    visibility: Option<TrackVisibility>,
}

impl<'a> PreparedTrackScope<'a> {
    pub fn track_ref(self) -> TrackRef {
        self.track_ref
    }
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        track_ref: TrackRef,
    ) -> Self {
        Self::with_track_observer(files, visibility, filter, track_ref, &mut |_| {})
    }

    fn with_track_observer(
        files: &'a [LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        track_ref: TrackRef,
        observe_track_filter: &mut impl FnMut(&LoadedTrack),
    ) -> Self {
        let track = track_ref.resolve(files);
        let track_visibility = track_ref
            .fi
            .get(&visibility.files)
            .filter(|file| file.enabled)
            .and_then(|file| track_ref.index.get(&file.tracks))
            .copied()
            .filter(|visibility| visibility.enabled)
            .filter(|_| {
                track.is_some_and(|track| {
                    observe_track_filter(track);
                    gt_filter::track_passes_filter(track, filter)
                })
            });
        Self {
            track_ref,
            track,
            visibility: track_visibility,
        }
    }

    pub fn track(self) -> Option<&'a LoadedTrack> {
        self.track
    }
    pub fn is_in_scope(self) -> bool {
        self.visibility.is_some()
    }
    pub fn category_visible(self, category: DataCategory) -> bool {
        self.visibility
            .is_some_and(|visibility| visibility.category_visible(category))
    }
}

#[derive(Clone, Copy)]
pub struct PreparedTrackEligibility<'a> {
    eligibility: MapEligibility<'a>,
    scope: PreparedTrackScope<'a>,
    hidden_ranges: &'a [Range<usize>],
}

impl<'a> PreparedTrackEligibility<'a> {
    fn new(eligibility: MapEligibility<'a>, scope: PreparedTrackScope<'a>) -> Self {
        Self {
            eligibility,
            scope,
            hidden_ranges: eligibility
                .query_matches
                .map_or(&[], |matches| matches.hidden_ranges(scope.track_ref)),
        }
    }

    pub fn scope(self) -> PreparedTrackScope<'a> {
        self.scope
    }
    pub fn track_ref(self) -> TrackRef {
        self.scope.track_ref
    }
    pub fn track(self) -> Option<&'a LoadedTrack> {
        self.scope.track()
    }
    pub fn is_in_scope(self) -> bool {
        self.scope.is_in_scope()
    }
    pub fn category_visible(self, category: DataCategory) -> bool {
        self.scope.category_visible(category)
    }

    pub fn available_fix_indices(self) -> impl DoubleEndedIterator<Item = PointIdx> + 'a {
        self.track()
            .filter(move |_| self.is_in_scope())
            .into_iter()
            .flat_map(move |track| {
                gt_filter::time_filtered_range(&track.points, self.eligibility.filter)
                    .map(PointIdx::new)
                    .filter(move |&point| self.fix_is_available(point))
            })
    }

    #[inline]
    pub fn fix_is_available(self, point: PointIdx) -> bool {
        self.is_in_scope()
            && self
                .track()
                .and_then(|track| point.get(&track.points))
                .is_some_and(|fix| self.fix_withheld(point, fix).is_none())
    }

    #[inline]
    fn fix_withheld(self, point: PointIdx, fix: &NavPoint) -> Option<EligibilityWithheld> {
        if QueryMatches::range_at(self.hidden_ranges, point.as_usize()).is_some() {
            Some(EligibilityWithheld::HiddenByQuery)
        } else if !self.eligibility.filter.contains_time(fix.tpv.time().utc()) {
            Some(EligibilityWithheld::OutsideTimeFilter)
        } else {
            None
        }
    }

    #[inline]
    pub fn element_eligibility(self, element_ref: MapElementRef) -> MapEligibilityResult {
        match self.resolve(element_ref) {
            ResolvedEligibility::Eligible(_) => MapEligibilityResult::Eligible,
            ResolvedEligibility::Missing => MapEligibilityResult::Missing,
            ResolvedEligibility::Withheld(reason) => MapEligibilityResult::Withheld(reason),
        }
    }

    #[inline]
    pub fn resolve_presence(
        self,
        element_ref: MapElementRef,
        display_mask: DisplayMask,
    ) -> MapPresenceResult<'a> {
        self.resolve(element_ref).with_display_mask(display_mask)
    }

    pub fn eligible_custom_markers(self) -> impl Iterator<Item = &'a CustomMarker> {
        self.track()
            .filter(|_| self.is_in_scope())
            .into_iter()
            .flat_map(|track| track.custom_markers.iter())
            .filter(move |marker| self.custom_marker_withheld(marker).is_none())
    }

    #[inline]
    fn custom_marker_withheld(self, marker: &CustomMarker) -> Option<EligibilityWithheld> {
        if !self.category_visible(DataCategory::CustomMarker) {
            Some(EligibilityWithheld::CategoryHidden)
        } else if !self.eligibility.filter.contains_time(marker.time) {
            Some(EligibilityWithheld::OutsideTimeFilter)
        } else {
            None
        }
    }

    pub fn eligible_generated_markers(self) -> impl Iterator<Item = &'a GeneratedMarker> {
        self.track()
            .filter(|_| self.is_in_scope())
            .into_iter()
            .flat_map(|track| track.generated_markers.iter())
            .filter(move |marker| self.generated_marker_withheld(marker).is_none())
    }

    #[inline]
    fn generated_marker_withheld(self, marker: &GeneratedMarker) -> Option<EligibilityWithheld> {
        if !self.category_visible(DataCategory::GeneratedMarker) {
            Some(EligibilityWithheld::CategoryHidden)
        } else if !self
            .eligibility
            .generated_marker_visibility
            .is_visible(self.track_ref(), marker.kind.tag())
        {
            Some(EligibilityWithheld::MarkerTypeHidden)
        } else if !self.eligibility.filter.contains_time(marker.time) {
            Some(EligibilityWithheld::OutsideTimeFilter)
        } else {
            None
        }
    }

    pub fn eligible_event_markers(self) -> impl Iterator<Item = &'a EventMarker> {
        self.track()
            .filter(|_| self.is_in_scope())
            .into_iter()
            .flat_map(|track| track.event_markers.iter())
            .filter(move |marker| self.event_marker_withheld(marker).is_none())
    }

    #[inline]
    fn event_marker_withheld(self, marker: &EventMarker) -> Option<EligibilityWithheld> {
        if !self.category_visible(DataCategory::EventMarker) {
            Some(EligibilityWithheld::CategoryHidden)
        } else if !self
            .eligibility
            .event_marker_visibility
            .is_visible(self.track_ref(), &marker.variant_path)
        {
            Some(EligibilityWithheld::MarkerTypeHidden)
        } else if !self.eligibility.filter.contains_time(marker.time) {
            Some(EligibilityWithheld::OutsideTimeFilter)
        } else {
            None
        }
    }

    #[inline]
    pub fn resolve(self, element_ref: MapElementRef) -> ResolvedEligibility<'a> {
        if element_ref.track() != self.track_ref() {
            return ResolvedEligibility::Missing;
        }
        let Some(element) = self
            .scope
            .track
            .and_then(|track| element_ref.resolve_in_track(track))
        else {
            return ResolvedEligibility::Missing;
        };
        let Some(track_visibility) = self.scope.visibility else {
            return ResolvedEligibility::Withheld(EligibilityWithheld::TrackNotShown);
        };
        // This is the semantic authority for point-like map elements. Keep the
        // family match exhaustive so a new identity cannot silently inherit an
        // existing policy.
        let withheld = match element {
            ResolvedElement::Fix(fix) | ResolvedElement::SatelliteReport { fix, .. } => {
                if !track_visibility.category_visible(DataCategory::Tpv) {
                    Some(EligibilityWithheld::CategoryHidden)
                } else {
                    element_ref
                        .fix()
                        .and_then(|reference| self.fix_withheld(reference.point, fix))
                }
            }
            ResolvedElement::CustomMarker(marker) => self.custom_marker_withheld(marker),
            ResolvedElement::GeneratedMarker(marker) => self.generated_marker_withheld(marker),
            ResolvedElement::EventMarker(marker) => self.event_marker_withheld(marker),
        };

        if let Some(reason) = withheld {
            ResolvedEligibility::Withheld(reason)
        } else {
            ResolvedEligibility::Eligible(EligibleElementRef {
                element_ref,
                element,
            })
        }
    }
}

#[derive(Clone, Copy)]
pub struct EligibleElementRef<'a> {
    element_ref: MapElementRef,
    element: ResolvedElement<'a>,
}

pub enum ResolvedEligibility<'a> {
    Eligible(EligibleElementRef<'a>),
    Missing,
    Withheld(EligibilityWithheld),
}

impl<'a> EligibleElementRef<'a> {
    pub fn element_ref(self) -> MapElementRef {
        self.element_ref
    }
    pub fn element(self) -> ResolvedElement<'a> {
        self.element
    }
}

impl<'a> ResolvedEligibility<'a> {
    #[inline]
    pub fn with_display_mask(self, display_mask: DisplayMask) -> MapPresenceResult<'a> {
        match self {
            Self::Missing => MapPresenceResult::Missing,
            Self::Withheld(reason) => {
                MapPresenceResult::Withheld(PresenceWithheld::Eligibility(reason))
            }
            Self::Eligible(eligible) => {
                if display_mask.is_visible(eligible.element.display_category()) {
                    MapPresenceResult::Present(PresentElementRef {
                        element_ref: eligible.element_ref,
                        element: eligible.element,
                    })
                } else {
                    MapPresenceResult::Withheld(PresenceWithheld::DisplayMasked)
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct PreparedTrackPresence<'a> {
    eligibility: PreparedTrackEligibility<'a>,
    display_mask: DisplayMask,
}

impl<'a> PreparedTrackPresence<'a> {
    pub fn is_in_scope(self) -> bool {
        self.eligibility.is_in_scope()
    }
    #[inline]
    pub fn resolve(self, element_ref: MapElementRef) -> MapPresenceResult<'a> {
        self.eligibility
            .resolve_presence(element_ref, self.display_mask)
    }
    pub fn draws(self, element_ref: MapElementRef) -> bool {
        matches!(self.resolve(element_ref), MapPresenceResult::Present(_))
    }
}

/// Why an existing semantically eligible element is absent from the rendered
/// map after display policy is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceWithheld {
    /// Pre-display semantic policy withheld the element.
    Eligibility(EligibilityWithheld),
    /// The element's render category is disabled by the display mask.
    DisplayMasked,
}

/// The result of resolving a raw map identity through all map policy.
#[derive(Clone, Copy)]
pub enum MapPresenceResult<'a> {
    /// The element is present and carries the proof required by present-only
    /// operations.
    Present(PresentElementRef<'a>),
    /// The raw identity no longer resolves against the loaded files.
    Missing,
    /// The element still exists but is withheld by policy.
    Withheld(PresenceWithheld),
}

/// Proof that a raw map identity resolves and is present under the complete
/// policy for this frame.
///
/// The fields are private so external code can obtain this value only by
/// resolving a [`MapElementRef`] through [`MapPresence`]. Do not persist this
/// value across frames. Persist the raw identity and resolve it again instead.
#[derive(Clone, Copy)]
pub struct PresentElementRef<'a> {
    element_ref: MapElementRef,
    element: ResolvedElement<'a>,
}

impl<'a> PresentElementRef<'a> {
    pub fn element_ref(self) -> MapElementRef {
        self.element_ref
    }

    pub fn element(self) -> ResolvedElement<'a> {
        self.element
    }
}

#[derive(Clone, Copy)]
struct PreparedTrackSlice<'a> {
    tracks: &'a [Option<TrackVisibility>],
    offsets: &'a [usize],
}

impl PreparedTrackSlice<'_> {
    #[inline]
    fn flat_track_index(self, track: TrackRef) -> Option<usize> {
        let fi = track.fi.as_usize();
        let (&start, &end) = (self.offsets.get(fi)?, self.offsets.get(fi + 1)?);
        let index = start.checked_add(track.index.as_usize())?;
        (index < end).then_some(index)
    }

    #[inline]
    fn track<'a>(
        self,
        track: TrackRef,
        eligibility: MapEligibility<'a>,
    ) -> Option<PreparedTrackEligibility<'a>> {
        Some(PreparedTrackEligibility::new(
            eligibility,
            PreparedTrackScope {
                track_ref: track,
                track: track.resolve(eligibility.files),
                visibility: *self.tracks.get(self.flat_track_index(track)?)?,
            },
        ))
    }
}

/// Complete point-like map policy for one frame: semantic eligibility plus the
/// render-side display mask.
#[derive(Clone, Copy)]
pub struct MapPresence<'a> {
    eligibility: MapEligibility<'a>,
    display_mask: DisplayMask,
    prepared: Option<PreparedTrackSlice<'a>>,
}

impl<'a> MapPresence<'a> {
    pub fn new(eligibility: MapEligibility<'a>, display_mask: DisplayMask) -> Self {
        Self {
            eligibility,
            display_mask,
            prepared: None,
        }
    }

    /// The semantic policy beneath this display-presence view.
    pub fn eligibility(self) -> MapEligibility<'a> {
        self.eligibility
    }

    /// The loaded recordings backing this frame policy.
    pub fn files(self) -> &'a [LoadedFile] {
        self.eligibility.files
    }

    /// Return the same semantic authority with a different display mask.
    pub fn with_display_mask(self, display_mask: DisplayMask) -> Self {
        Self {
            display_mask,
            ..self
        }
    }

    /// Rebind the last-query policy while preserving every other policy source.
    pub fn with_query_matches<'b>(self, query_matches: Option<&'b QueryMatches>) -> MapPresence<'b>
    where
        'a: 'b,
    {
        MapPresence {
            eligibility: MapEligibility {
                files: self.eligibility.files,
                visibility: self.eligibility.visibility,
                filter: self.eligibility.filter,
                query_matches,
                generated_marker_visibility: self.eligibility.generated_marker_visibility,
                event_marker_visibility: self.eligibility.event_marker_visibility,
            },
            display_mask: self.display_mask,
            prepared: self.prepared,
        }
    }

    /// Resolve `element_ref` through semantic policy and the display mask.
    #[inline]
    pub fn resolve(self, element_ref: MapElementRef) -> MapPresenceResult<'a> {
        self.prepare_track(element_ref.track()).resolve(element_ref)
    }

    pub fn prepare_track(self, track_ref: TrackRef) -> PreparedTrackPresence<'a> {
        PreparedTrackPresence {
            eligibility: self
                .prepared
                .and_then(|prepared| prepared.track(track_ref, self.eligibility))
                .unwrap_or_else(|| self.eligibility.prepare_track(track_ref)),
            display_mask: self.display_mask,
        }
    }

    pub fn display_mask(self) -> DisplayMask {
        self.display_mask
    }

    /// Whether the map draws the element `element_ref` addresses.
    pub fn draws(self, element_ref: MapElementRef) -> bool {
        matches!(self.resolve(element_ref), MapPresenceResult::Present(_))
    }

    /// Compatibility classification for existing map consumers.
    pub fn point_visibility(self, element_ref: MapElementRef) -> PointVisibility {
        match self.resolve(element_ref) {
            MapPresenceResult::Present(_) => PointVisibility::Shown,
            MapPresenceResult::Missing => PointVisibility::NoSuchElement,
            MapPresenceResult::Withheld(PresenceWithheld::DisplayMasked) => {
                PointVisibility::CategoryHidden
            }
            MapPresenceResult::Withheld(PresenceWithheld::Eligibility(reason)) => match reason {
                EligibilityWithheld::TrackNotShown => PointVisibility::TrackNotShown,
                EligibilityWithheld::CategoryHidden => PointVisibility::CategoryHidden,
                EligibilityWithheld::MarkerTypeHidden => PointVisibility::MarkerTypeHidden,
                EligibilityWithheld::HiddenByQuery => PointVisibility::HiddenByQuery,
                EligibilityWithheld::OutsideTimeFilter => PointVisibility::OutsideTimeFilter,
            },
        }
    }
}

/// The borrowed element that a [`MapElementRef`] resolves to.
#[derive(Clone, Copy)]
pub enum ResolvedElement<'a> {
    Fix(&'a NavPoint),
    SatelliteReport {
        fix: &'a NavPoint,
        report: &'a Satellites,
    },
    CustomMarker(&'a CustomMarker),
    GeneratedMarker(&'a GeneratedMarker),
    EventMarker(&'a EventMarker),
}

impl MapElementRef {
    /// Resolve this typed identity against the currently loaded recordings.
    pub fn resolve<'a>(self, files: &'a [LoadedFile]) -> Option<ResolvedElement<'a>> {
        self.resolve_in_track(self.track().resolve(files)?)
    }

    #[inline]
    fn resolve_in_track(self, track: &LoadedTrack) -> Option<ResolvedElement<'_>> {
        match self {
            Self::Fix(reference) => reference.point.get(&track.points).map(ResolvedElement::Fix),
            Self::SatelliteReport(reference) => {
                let fix = reference.point.get(&track.points)?;
                Some(ResolvedElement::SatelliteReport {
                    fix,
                    report: fix.satellites.as_ref()?,
                })
            }
            Self::CustomMarker(reference) => reference
                .index
                .get(&track.custom_markers)
                .map(ResolvedElement::CustomMarker),
            Self::GeneratedMarker(reference) => reference
                .index
                .get(&track.generated_markers)
                .map(ResolvedElement::GeneratedMarker),
            Self::EventMarker(reference) => reference
                .index
                .get(&track.event_markers)
                .map(ResolvedElement::EventMarker),
        }
    }
}

impl ResolvedElement<'_> {
    pub fn display_category(self) -> DisplayCategory {
        match self {
            Self::Fix(fix) | Self::SatelliteReport { fix, .. } if fix.is_navigation_estimate() => {
                DisplayCategory::GhostFixes
            }
            Self::Fix(_) | Self::SatelliteReport { .. } => DisplayCategory::TrackPoints,
            Self::CustomMarker(_) => DisplayCategory::CustomMarkers,
            Self::GeneratedMarker(_) => DisplayCategory::GeneratedMarkers,
            Self::EventMarker(_) => DisplayCategory::EventMarkers,
        }
    }

    /// The timestamp that the time filter compares against its window.
    pub fn time(self) -> DateTime<Utc> {
        match self {
            Self::Fix(fix) | Self::SatelliteReport { fix, .. } => fix.tpv.time().utc(),
            Self::CustomMarker(marker) => marker.time,
            Self::GeneratedMarker(marker) => marker.time,
            Self::EventMarker(marker) => marker.time,
        }
    }
}

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeDelta;
    use gt_types::fixtures::FixKind;
    use gt_types::{
        CustomMarkerIdx, CustomMarkerRef, EventMarkerIdx, EventMarkerRef, FileSource,
        GeneratedMarkerIdx, GeneratedMarkerKindTag, GeneratedMarkerRef, GpsTime, Latitude,
        Longitude, SolutionState, TimePositionVelocity,
    };

    use super::*;
    use crate::test_util::{self, ScopeFixture};

    fn vis_all() -> TrackDataVisibility {
        TrackDataVisibility::from_loaded(&test_util::one_track_file())
    }

    #[test]
    fn everything_enabled_is_in_scope() {
        let files = test_util::one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        assert!(track_in_scope(&files, &vis, &filter, test_util::track0()).is_some());
    }

    #[rstest::rstest]
    #[case::file_disabled(|vis: &mut TrackDataVisibility| vis.files[0].enabled = false)]
    #[case::track_disabled(|vis: &mut TrackDataVisibility| vis.files[0].tracks[0].enabled = false)]
    fn disabled_nodes_are_out_of_scope(#[case] disable: fn(&mut TrackDataVisibility)) {
        let files = test_util::one_track_file();
        let mut vis = vis_all();
        disable(&mut vis);
        assert!(
            track_in_scope(&files, &vis, &GlobalFilter::default(), test_util::track0()).is_none()
        );
    }

    #[test]
    fn failing_the_track_filter_is_out_of_scope() {
        let files = test_util::one_track_file();
        let filter = GlobalFilter::default().with_minimum_duration(Some(TimeDelta::hours(1)));
        assert!(track_in_scope(&files, &vis_all(), &filter, test_util::track0()).is_none());
    }

    /// A [`TrackRef`] left over from before a file shrank addresses nothing and
    /// is out of scope.
    #[test]
    fn stale_indices_are_out_of_scope() {
        let files = test_util::one_track_file();
        let vis = vis_all();
        let filter = GlobalFilter::default();
        let stale = TrackRef::new(FileIdx::new(0), TrackIdx::new(7));
        assert!(track_in_scope(&files, &vis, &filter, stale).is_none());
    }

    #[rstest::rstest]
    #[case(DataCategory::Track)]
    #[case(DataCategory::Tpv)]
    #[case(DataCategory::SatelliteReport)]
    #[case(DataCategory::CustomMarker)]
    #[case(DataCategory::GeneratedMarker)]
    #[case(DataCategory::EventMarker)]
    fn category_visible_reads_exactly_its_flag(#[case] category: DataCategory) {
        let mut tv = TrackVisibility::all_visible();
        assert!(tv.category_visible(category));
        tv.set_category_visible(category, false);
        assert!(!tv.category_visible(category));
        // Exactly one category flag changed: every other category still on.
        let others = [
            DataCategory::Track,
            DataCategory::Tpv,
            DataCategory::SatelliteReport,
            DataCategory::CustomMarker,
            DataCategory::GeneratedMarker,
            DataCategory::EventMarker,
        ]
        .into_iter()
        .filter(|&c| c != category)
        .all(|c| tv.category_visible(c));
        assert!(others);
    }

    #[test]
    fn presence_distinguishes_missing_semantic_policy_and_display_masking() {
        let mut fixture = ScopeFixture::all_drawn();
        let satellite = MapElementRef::SatelliteReport(gt_types::FixRef::new(
            test_util::track0(),
            PointIdx::new(0),
        ));
        let presence = fixture.scope();

        assert_eq!(
            presence.eligibility().element_eligibility(satellite),
            MapEligibilityResult::Eligible
        );
        match presence.resolve(satellite) {
            MapPresenceResult::Present(present) => assert_eq!(present.element_ref(), satellite),
            MapPresenceResult::Missing | MapPresenceResult::Withheld(_) => {
                panic!("satellite report should be present")
            }
        }

        fixture
            .display_mask
            .set_visible(DisplayCategory::TrackPoints, false);
        let masked = fixture.scope();
        assert_eq!(
            masked.eligibility().element_eligibility(satellite),
            MapEligibilityResult::Eligible,
            "display masking must not leak into semantic eligibility"
        );
        assert!(matches!(
            masked.resolve(satellite),
            MapPresenceResult::Withheld(PresenceWithheld::DisplayMasked)
        ));

        let stale = MapElementRef::Fix(gt_types::FixRef::new(
            test_util::track0(),
            PointIdx::new(test_util::POINT_COUNT + 1),
        ));
        assert!(matches!(masked.resolve(stale), MapPresenceResult::Missing));
    }

    #[test]
    fn stale_typed_marker_refs_resolve_as_missing() {
        let files = test_util::one_track_file();
        let track = test_util::track0();
        let stale_index = 10_000;
        let stale = [
            MapElementRef::CustomMarker(CustomMarkerRef::new(
                track,
                CustomMarkerIdx::new(stale_index),
            )),
            MapElementRef::GeneratedMarker(GeneratedMarkerRef::new(
                track,
                GeneratedMarkerIdx::new(stale_index),
            )),
            MapElementRef::EventMarker(EventMarkerRef::new(
                track,
                EventMarkerIdx::new(stale_index),
            )),
        ];

        for element in stale {
            assert!(element.resolve(&files).is_none());
        }
    }

    /// `track_shown` requires the file, the track, and the track-line
    /// toggle. Any one of them off hides the track. Out-of-range refs are
    /// simply not shown.
    #[rstest::rstest]
    #[case::all_on(true, true, true, true)]
    #[case::file_disabled(false, true, true, false)]
    #[case::track_disabled(true, false, true, false)]
    #[case::trackline_hidden(true, true, false, false)]
    fn track_shown_needs_file_track_and_line(
        #[case] file_enabled: bool,
        #[case] track_enabled: bool,
        #[case] track_visible: bool,
        #[case] expected: bool,
    ) {
        let mut tv = TrackVisibility::all_visible();
        tv.enabled = track_enabled;
        tv.set_category_visible(DataCategory::Track, track_visible);
        let vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: file_enabled,
                tracks: vec![tv],
            }],
        };
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        assert_eq!(vis.track_shown(track_ref), expected);
        assert!(
            !vis.track_shown(TrackRef::new(FileIdx::new(1), TrackIdx::new(0))),
            "an out-of-range ref is never shown"
        );
    }

    /// The tree hides an event marker by its own path or by a path above it,
    /// and a generated marker by its own kind. A fix has no marker type.
    #[rstest::rstest]
    #[case::event_marker_with_its_path_hidden(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| fixture.hide_event_marker_path(test_util::EVENT_MARKER_PATH),
        PointVisibility::MarkerTypeHidden
    )]
    #[case::event_marker_under_a_hidden_parent_path(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| {
            fixture.hide_event_marker_path(test_util::EVENT_MARKER_PARENT_PATH);
        },
        PointVisibility::MarkerTypeHidden
    )]
    #[case::event_marker_above_a_hidden_child_path(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| fixture.hide_event_marker_path("power/boot/cold"),
        PointVisibility::Shown
    )]
    #[case::event_marker_while_a_text_prefix_of_its_parent_path_is_hidden(
        test_util::event_marker(),
        |fixture: &mut ScopeFixture| fixture.hide_event_marker_path("pow"),
        PointVisibility::Shown
    )]
    #[case::generated_marker_of_a_hidden_kind(
        test_util::generated_marker(),
        |fixture: &mut ScopeFixture| {
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixLost);
        },
        PointVisibility::MarkerTypeHidden
    )]
    #[case::generated_marker_while_another_kind_is_hidden(
        test_util::generated_marker(),
        |fixture: &mut ScopeFixture| {
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixRegained);
        },
        PointVisibility::Shown
    )]
    #[case::fix_while_every_marker_type_is_hidden(
        test_util::point(0),
        |fixture: &mut ScopeFixture| {
            fixture.hide_event_marker_path(test_util::EVENT_MARKER_PARENT_PATH);
            fixture.hide_generated_marker_kind(GeneratedMarkerKindTag::GnssFixLost);
        },
        PointVisibility::Shown
    )]
    fn the_tree_hides_a_marker_by_its_path_its_parent_path_or_its_kind(
        #[case] point: MapElementRef,
        #[case] hide: fn(&mut ScopeFixture),
        #[case] expected: PointVisibility,
    ) {
        let mut fixture = ScopeFixture::all_drawn();
        hide(&mut fixture);

        assert_eq!(fixture.scope().point_visibility(point), expected);
    }

    #[test]
    fn ghost_fixes_track_ghost_fixes_display_category() {
        let mut ghost_point = NavPoint::new(
            TimePositionVelocity::builder()
                .time(GpsTime::from_utc(test_util::start()))
                .lat(Latitude::new(55.0))
                .lon(Longitude::new(12.0))
                .build(),
            None,
        );
        ghost_point.navigation.state = Some(SolutionState::NoFix);
        let real_point = gt_types::fixtures::nav_point(
            test_util::start() + TimeDelta::seconds(1),
            Latitude::new(55.0),
            Longitude::new(12.0),
            FixKind::Measured,
        );
        let track = gt_track_builder::build_loaded_file(
            "test.gtd".to_owned(),
            &[ghost_point, real_point],
            &[],
            Vec::new(),
            Vec::new(),
            &[],
            &gt_track_builder::SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("test.gtd")),
            gt_track_builder::FileMeta::default(),
            Vec::new(),
        );
        let files = vec![track];
        let vis = TrackDataVisibility::from_loaded(&files);
        let filter = GlobalFilter::default();

        let ghost_ref =
            MapElementRef::Fix(gt_types::FixRef::new(test_util::track0(), PointIdx::new(0)));
        let real_ref =
            MapElementRef::Fix(gt_types::FixRef::new(test_util::track0(), PointIdx::new(1)));

        let mut mask = DisplayMask::default();
        let event_marker_visibility = EventMarkerVisibility::default();
        let generated_marker_visibility = GeneratedMarkerVisibility::default();
        let eligibility = MapEligibility::new(
            &files,
            &vis,
            &filter,
            None,
            &generated_marker_visibility,
            &event_marker_visibility,
        );
        let scope = eligibility.with_display_mask(mask);
        assert_eq!(scope.point_visibility(ghost_ref), PointVisibility::Shown);
        assert_eq!(scope.point_visibility(real_ref), PointVisibility::Shown);

        mask.set_visible(DisplayCategory::GhostFixes, false);
        let scope = scope.with_display_mask(mask);
        assert_eq!(
            scope.point_visibility(ghost_ref),
            PointVisibility::CategoryHidden
        );
        assert_eq!(scope.point_visibility(real_ref), PointVisibility::Shown);

        mask = DisplayMask::default();
        mask.solo(DisplayCategory::GhostFixes);
        let scope = scope.with_display_mask(mask);
        assert_eq!(scope.point_visibility(ghost_ref), PointVisibility::Shown);
        assert_eq!(
            scope.point_visibility(real_ref),
            PointVisibility::CategoryHidden
        );
    }
}
