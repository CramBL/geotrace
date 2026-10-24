use std::cmp::Ordering;

use chrono::{DateTime, NaiveTime, Utc};
use nmea::sentences::gns::NavigationStatus;
use nmea::sentences::gsa::GsaMode2;
use nmea::sentences::rmc::{RmcNavigationStatus, RmcStatusOfFix};
use nmea::sentences::{FixType, GsaData, VtgData};
use nmea::{ParseResult, SentenceType};

use crate::{
    Angle, NavFix, NavFixTime, PositionProvenance, RecordedFixTimestamps, SolutionDimension,
    SolutionMethod, SolutionState, Velocity,
};

use super::time::ReceiverSegment;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub(super) enum NavigationSource {
    Gga = 2,
    Gll = 0,
    Gns = 3,
    Rmc = 1,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NavigationFields {
    pub source: NavigationSource,
    pub time_of_day: Option<NaiveTime>,
    pub receiver_time: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub state: Option<SolutionState>,
    pub method: Option<SolutionMethod>,
    pub heading: Option<f64>,
    pub speed_knots: Option<f64>,
    pub hdop: Option<f64>,
    pub used_count: Option<u32>,
}

impl NavigationFields {
    pub fn from_parsed(parsed: ParseResult) -> Option<Self> {
        let mut fields = Self {
            source: NavigationSource::Gll,
            time_of_day: None,
            receiver_time: None,
            latitude: None,
            longitude: None,
            state: None,
            method: None,
            heading: None,
            speed_knots: None,
            hdop: None,
            used_count: None,
        };
        match parsed {
            ParseResult::GGA(data) => {
                fields.source = NavigationSource::Gga;
                fields.time_of_day = data.fix_time;
                fields.latitude = data.latitude;
                fields.longitude = data.longitude;
                let solution = data.fix_type.map(Solution::from_fix_type);
                fields.state = solution.map(|value| value.state);
                fields.method = solution.and_then(|value| value.method);
                fields.hdop = data.hdop.map(f64::from);
                fields.used_count = data.fix_satellites;
            }
            ParseResult::GNS(data) => {
                fields.source = NavigationSource::Gns;
                fields.time_of_day = data.fix_time;
                fields.latitude = data.lat;
                fields.longitude = data.lon;
                let solution = Solution::from_fix_type(FixType::from(data.faa_modes));
                fields.state = Some(
                    if data.nav_status == Some(NavigationStatus::NotValidForNavigation) {
                        SolutionState::NoFix
                    } else {
                        solution.state
                    },
                );
                fields.method = solution.method;
                fields.hdop = data.hdop.map(f64::from);
                fields.used_count = Some(u32::from(data.nsattelites));
            }
            ParseResult::GLL(data) => {
                fields.time_of_day = data.fix_time;
                fields.latitude = data.latitude;
                fields.longitude = data.longitude;
                let solution = data
                    .faa_mode
                    .map(FixType::from)
                    .map(Solution::from_fix_type);
                fields.state = Some(if !data.valid {
                    SolutionState::NoFix
                } else {
                    solution.map_or(SolutionState::Fix, |value| value.state)
                });
                fields.method = solution.and_then(|value| value.method);
            }
            ParseResult::RMC(data) => {
                fields.source = NavigationSource::Rmc;
                fields.time_of_day = data.fix_time;
                fields.receiver_time = data
                    .fix_date
                    .zip(data.fix_time)
                    .map(|(date, time)| date.and_time(time).and_utc());
                fields.latitude = data.lat;
                fields.longitude = data.lon;
                let solution = data
                    .faa_mode
                    .map(FixType::from)
                    .map(Solution::from_fix_type)
                    .or_else(|| data.nav_status.and_then(Solution::from_navigation_status));
                fields.state = Some(
                    if data.status_of_fix == RmcStatusOfFix::Invalid
                        || data.nav_status == Some(RmcNavigationStatus::NotValid)
                    {
                        SolutionState::NoFix
                    } else {
                        solution.map_or(SolutionState::Fix, |value| value.state)
                    },
                );
                fields.method =
                    solution
                        .and_then(|value| value.method)
                        .or(match data.status_of_fix {
                            RmcStatusOfFix::Differential => Some(SolutionMethod::Differential),
                            RmcStatusOfFix::Autonomous => Some(SolutionMethod::Autonomous),
                            RmcStatusOfFix::Invalid => None,
                        });
                fields.heading = data.true_course.map(f64::from);
                fields.speed_knots = data.speed_over_ground.map(f64::from);
            }
            _ => return None,
        }
        Some(fields)
    }
}

#[derive(Clone, Debug)]
pub(super) struct NavigationEpoch {
    pub segment: ReceiverSegment,
    pub receiver_reliable: bool,
    pub date_attribution_allowed: bool,
    pub receiver_time: Option<DateTime<Utc>>,
    pub time_of_day: Option<NaiveTime>,
    pub host_time: Option<DateTime<Utc>>,
    pub first_line: u64,
    pub last_line: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub bracket_first_sequence: u64,
    pub bracket_last_sequence: u64,
    pub sentences: Vec<SentenceType>,
    position: Option<PositionSample>,
    state: StateEvidence,
    method: Option<RankedValue<SolutionMethod>>,
    heading: Option<RankedValue<f64>>,
    speed: Option<RankedValue<f64>>,
    hdop: Option<RankedValue<f64>>,
    used_count: Option<RankedValue<u32>>,
    pub dimension: Option<SolutionDimension>,
    dimension_conflict: bool,
    pdop: DopEvidence,
    vdop: DopEvidence,
    gsa_hdop: DopEvidence,
}

impl NavigationEpoch {
    fn method_specificity(method: SolutionMethod) -> MethodSpecificity {
        match method {
            SolutionMethod::RtkFixed => MethodSpecificity::RtkFixed,
            SolutionMethod::RtkFloat => MethodSpecificity::RtkFloat,
            SolutionMethod::Differential => MethodSpecificity::Differential,
            SolutionMethod::Hybrid => MethodSpecificity::Hybrid,
            SolutionMethod::Autonomous => MethodSpecificity::Autonomous,
            SolutionMethod::DeadReckoning => MethodSpecificity::DeadReckoning,
            SolutionMethod::Other => MethodSpecificity::Other,
        }
    }

    pub fn new(input: NavigationInput) -> Self {
        let mut epoch = Self {
            segment: input.segment,
            receiver_reliable: input.receiver_reliable,
            date_attribution_allowed: input.date_attribution_allowed,
            receiver_time: input.fields.receiver_time,
            time_of_day: input.fields.time_of_day,
            host_time: input.host_time,
            first_line: input.line,
            last_line: input.line,
            first_sequence: input.sequence,
            last_sequence: input.sequence,
            bracket_first_sequence: input.sequence,
            bracket_last_sequence: input.sequence,
            sentences: Vec::new(),
            position: None,
            state: StateEvidence::Missing,
            method: None,
            heading: None,
            speed: None,
            hdop: None,
            used_count: None,
            dimension: None,
            dimension_conflict: false,
            pdop: DopEvidence::Missing,
            vdop: DopEvidence::Missing,
            gsa_hdop: DopEvidence::Missing,
        };
        epoch.merge(input);
        epoch
    }

    pub fn merge(&mut self, input: NavigationInput) -> bool {
        self.receiver_reliable |= input.receiver_reliable;
        self.receiver_time = self.receiver_time.or(input.fields.receiver_time);
        self.host_time = self.host_time.into_iter().chain(input.host_time).min();
        self.first_line = self.first_line.min(input.line);
        self.last_line = self.last_line.max(input.line);
        self.first_sequence = self.first_sequence.min(input.sequence);
        self.last_sequence = self.last_sequence.max(input.sequence);
        if !self.sentences.contains(&input.sentence) {
            self.bracket_first_sequence = self.bracket_first_sequence.min(input.sequence);
            self.bracket_last_sequence = self.bracket_last_sequence.max(input.sequence);
            self.sentences.push(input.sentence);
        }
        let fields = input.fields;
        let source = fields.source;
        let mut conflict = self.merge_position(fields);
        conflict |= self.state.merge(fields.state);
        if let Some(method) = fields.method {
            let old_rank = self.method.map(|old| Self::method_specificity(old.value));
            if old_rank.is_none_or(|rank| Self::method_specificity(method) > rank) {
                self.method = Some(RankedValue {
                    source,
                    value: method,
                });
            }
        }
        conflict |= RankedValue::merge(
            &mut self.heading,
            fields
                .heading
                .filter(|value| value.is_finite())
                .map(|value| value.rem_euclid(FULL_CIRCLE_DEGREES)),
            source,
            |a, b| a.total_cmp(&b),
        );
        conflict |= RankedValue::merge(
            &mut self.speed,
            fields
                .speed_knots
                .filter(|value| value.is_finite() && *value >= 0.0),
            source,
            |a, b| a.total_cmp(&b),
        );
        conflict |= RankedValue::merge(
            &mut self.hdop,
            fields
                .hdop
                .filter(|value| value.is_finite() && *value >= 0.0),
            source,
            |a, b| a.total_cmp(&b),
        );
        conflict |= RankedValue::merge(&mut self.used_count, fields.used_count, source, |a, b| {
            a.cmp(&b)
        });
        conflict
    }

    pub fn merge_gsa(&mut self, data: &GsaData) -> bool {
        let mut conflict = self.pdop.merge(data.pdop)
            | self.vdop.merge(data.vdop)
            | self.gsa_hdop.merge(data.hdop);
        let dimension = match data.mode2 {
            GsaMode2::Fix2D => Some(SolutionDimension::TwoD),
            GsaMode2::Fix3D => Some(SolutionDimension::ThreeD),
            GsaMode2::NoFix => None,
        };
        conflict |= self.state.merge(Some(if data.mode2 == GsaMode2::NoFix {
            SolutionState::NoFix
        } else {
            SolutionState::Fix
        }));
        if self
            .dimension
            .zip(dimension)
            .is_some_and(|(previous, current)| previous != current)
        {
            self.dimension_conflict = true;
            conflict = true;
        } else if self.dimension.is_none() {
            self.dimension = dimension;
        }
        conflict
    }

    pub fn merge_velocity(&mut self, data: &VtgData) {
        RankedValue::merge(
            &mut self.heading,
            data.true_course
                .map(f64::from)
                .filter(|value| value.is_finite())
                .map(|value| value.rem_euclid(FULL_CIRCLE_DEGREES)),
            NavigationSource::Gll,
            |a, b| a.total_cmp(&b),
        );
        RankedValue::merge(
            &mut self.speed,
            data.speed_over_ground
                .map(f64::from)
                .filter(|value| value.is_finite() && *value >= 0.0),
            NavigationSource::Gll,
            |a, b| a.total_cmp(&b),
        );
    }

    pub fn into_fix(self) -> Option<NavFix> {
        let time = NavFixTime::from_recorded(RecordedFixTimestamps {
            gps: self
                .receiver_reliable
                .then_some(self.receiver_time)
                .flatten(),
            sys: self.host_time,
        })?;
        let position = self.position.map(|entry| entry.coordinates);
        let (lat, lon) = position.unwrap_or((f64::NAN, f64::NAN));
        let state = self.state.reported();
        let solution_available = matches!(
            state,
            Some(SolutionState::Fix | SolutionState::EstimatedOnly)
        );
        Some(
            NavFix::builder()
                .time(time)
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .maybe_heading(self.heading.map(|entry| Angle::degrees(entry.value)))
                .maybe_speed(self.speed.map(|entry| Velocity::knot(entry.value)))
                .maybe_state(state)
                .maybe_method(
                    solution_available
                        .then_some(self.method)
                        .flatten()
                        .map(|entry| entry.value),
                )
                .maybe_dimension(
                    (solution_available && !self.dimension_conflict)
                        .then_some(self.dimension)
                        .flatten(),
                )
                .maybe_used_count(self.used_count.map(|entry| entry.value))
                .maybe_hdop(
                    self.hdop
                        .map(|entry| entry.value)
                        .or(self.gsa_hdop.reported()),
                )
                .maybe_pdop(self.pdop.reported())
                .maybe_vdop(self.vdop.reported())
                .position_provenance(position.map(|_| PositionProvenance::SourceProvided))
                .build(),
        )
    }

    fn merge_position(&mut self, fields: NavigationFields) -> bool {
        let Some(coordinates) = fields
            .latitude
            .zip(fields.longitude)
            .filter(|(lat, lon)| lat.is_finite() && lon.is_finite())
        else {
            return false;
        };
        let candidate = PositionSample {
            coordinates,
            source: fields.source,
            state: fields.state,
            method: fields.method,
        };
        let conflict = self.position.as_ref().is_some_and(|previous| {
            candidate.quality() == previous.quality()
                && candidate
                    .coordinates
                    .0
                    .total_cmp(&previous.coordinates.0)
                    .then(candidate.coordinates.1.total_cmp(&previous.coordinates.1))
                    != Ordering::Equal
        });
        if self.position.as_ref().is_none_or(|previous| {
            candidate.quality() > previous.quality()
                || (candidate.quality() == previous.quality()
                    && candidate
                        .coordinates
                        .0
                        .total_cmp(&previous.coordinates.0)
                        .then(candidate.coordinates.1.total_cmp(&previous.coordinates.1))
                        == Ordering::Less)
        }) {
            self.position = Some(candidate);
        }
        conflict
    }
}

#[derive(Clone, Copy)]
pub(super) struct NavigationInput {
    pub segment: ReceiverSegment,
    pub receiver_reliable: bool,
    pub date_attribution_allowed: bool,
    pub fields: NavigationFields,
    pub host_time: Option<DateTime<Utc>>,
    pub line: u64,
    pub sequence: u64,
    pub sentence: SentenceType,
}

#[derive(Clone, Copy)]
struct Solution {
    state: SolutionState,
    method: Option<SolutionMethod>,
}

impl Solution {
    fn from_navigation_status(status: RmcNavigationStatus) -> Option<Self> {
        let fix_type = match status {
            RmcNavigationStatus::Autonomous => FixType::Gps,
            RmcNavigationStatus::Differential => FixType::DGps,
            RmcNavigationStatus::Estimated => FixType::Estimated,
            RmcNavigationStatus::Manual => FixType::Manual,
            RmcNavigationStatus::Simulator => FixType::Simulation,
            RmcNavigationStatus::NotValid => FixType::Invalid,
            RmcNavigationStatus::Valid => return None,
        };
        Some(Self::from_fix_type(fix_type))
    }

    fn from_fix_type(fix: FixType) -> Self {
        let (state, method) = match fix {
            FixType::Invalid => (SolutionState::NoFix, None),
            FixType::Gps => (SolutionState::Fix, Some(SolutionMethod::Autonomous)),
            FixType::DGps => (SolutionState::Fix, Some(SolutionMethod::Differential)),
            FixType::Pps => (SolutionState::Fix, Some(SolutionMethod::Other)),
            FixType::Rtk => (SolutionState::Fix, Some(SolutionMethod::RtkFixed)),
            FixType::FloatRtk => (SolutionState::Fix, Some(SolutionMethod::RtkFloat)),
            FixType::Estimated => (
                SolutionState::EstimatedOnly,
                Some(SolutionMethod::DeadReckoning),
            ),
            FixType::Manual | FixType::Simulation => {
                (SolutionState::EstimatedOnly, Some(SolutionMethod::Other))
            }
        };
        Self { state, method }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
enum MethodSpecificity {
    Autonomous = 3,
    DeadReckoning = 2,
    Differential = 5,
    Hybrid = 4,
    Other = 1,
    RtkFixed = 7,
    RtkFloat = 6,
}

#[derive(Clone, Copy, Debug, Default)]
enum StateEvidence {
    Conflicting,
    #[default]
    Missing,
    Reported(SolutionState),
}

impl StateEvidence {
    fn merge(&mut self, value: Option<SolutionState>) -> bool {
        let Some(value) = value else {
            return false;
        };
        match *self {
            Self::Missing => {
                *self = Self::Reported(value);
                false
            }
            Self::Reported(previous) if previous != value => {
                *self = Self::Conflicting;
                true
            }
            Self::Reported(_) | Self::Conflicting => false,
        }
    }

    fn reported(self) -> Option<SolutionState> {
        match self {
            Self::Conflicting => Some(SolutionState::Unknown),
            Self::Missing => None,
            Self::Reported(state) => Some(state),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum DopEvidence {
    Conflicting,
    #[default]
    Missing,
    Reported(f64),
}

impl DopEvidence {
    fn merge(&mut self, value: Option<f32>) -> bool {
        let Some(value) = value
            .map(f64::from)
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            return false;
        };
        match *self {
            Self::Missing => {
                *self = Self::Reported(value);
                false
            }
            Self::Reported(previous) if previous.total_cmp(&value) != Ordering::Equal => {
                *self = Self::Conflicting;
                true
            }
            Self::Reported(_) | Self::Conflicting => false,
        }
    }

    fn reported(self) -> Option<f64> {
        match self {
            Self::Reported(value) => Some(value),
            Self::Missing | Self::Conflicting => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RankedValue<T> {
    source: NavigationSource,
    value: T,
}

impl<T: Copy> RankedValue<T> {
    fn merge(
        slot: &mut Option<Self>,
        value: Option<T>,
        source: NavigationSource,
        tie: impl Fn(T, T) -> Ordering,
    ) -> bool {
        let Some(value) = value else {
            return false;
        };
        let conflict = slot
            .as_ref()
            .is_some_and(|old| old.source == source && tie(value, old.value) != Ordering::Equal);
        if slot.as_ref().is_none_or(|old| {
            source > old.source || (source == old.source && tie(value, old.value) == Ordering::Less)
        }) {
            *slot = Some(Self { source, value });
        }
        conflict
    }
}

#[derive(Clone, Copy, Debug)]
struct PositionSample {
    coordinates: (f64, f64),
    source: NavigationSource,
    state: Option<SolutionState>,
    method: Option<SolutionMethod>,
}

impl PositionSample {
    fn quality(
        &self,
    ) -> (
        PositionReliability,
        Option<MethodSpecificity>,
        NavigationSource,
    ) {
        let reliability = match self.state {
            Some(SolutionState::Fix) => PositionReliability::Measured,
            Some(SolutionState::EstimatedOnly) => PositionReliability::Estimated,
            Some(SolutionState::NoFix | SolutionState::Unknown) => PositionReliability::Unavailable,
            None => PositionReliability::Unreported,
        };
        (
            reliability,
            self.method.map(NavigationEpoch::method_specificity),
            self.source,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
enum PositionReliability {
    Estimated = 2,
    Measured = 3,
    Unavailable = 0,
    Unreported = 1,
}

const FULL_CIRCLE_DEGREES: f64 = 360.0;
