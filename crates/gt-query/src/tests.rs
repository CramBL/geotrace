use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use gt_types::{DisplayMode, FileIdx, TrackIdx, TrackRef};
use rstest::rstest;

use super::*;
use crate::test_util::{self, TestProvider};

mod aggregates;
mod arithmetic;
mod cancellation;
mod channel_evaluation;
mod channel_source;
mod channels;
mod columns;
mod display_mode;
mod error_messages;
mod matching;
mod params;
mod parsing_and_formatting;
mod powers;
mod properties;
mod ratio_metrics;
mod roots;
mod windows;

const UC1: &str = "points
| window 10
| where spread(heading) <= 10 deg
    and avg(accel) >= 0.3 m/s2
    and avg(velocity) > 30 km/h
| draw
| table time, velocity, heading, accel";
