use std::collections::BTreeMap;

use gt_types::satellites::{Constellation, Slip};
use gt_types::{SatelliteQueryScope, SatelliteReportMetadata};
use strum::IntoEnumIterator as _;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObservationEpoch {
    pub point_index: usize,
    pub observation_index: usize,
    pub metadata: SatelliteReportMetadata,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiagnosticRates {
    pub all: Option<f64>,
    constellations: BTreeMap<Constellation, Option<f64>>,
}

impl DiagnosticRates {
    pub fn for_scopes(mut calculate: impl FnMut(SatelliteQueryScope) -> Option<f64>) -> Self {
        Self {
            all: calculate(SatelliteQueryScope::Global),
            constellations: Constellation::iter()
                .map(|constellation| {
                    (
                        constellation,
                        calculate(SatelliteQueryScope::Constellation(constellation)),
                    )
                })
                .collect(),
        }
    }

    pub fn get(&self, scope: SatelliteQueryScope) -> Option<f64> {
        match scope {
            SatelliteQueryScope::Global => self.all,
            SatelliteQueryScope::Constellation(constellation) => {
                self.constellations.get(&constellation).copied().flatten()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlipEpoch {
    pub source: ObservationEpoch,
    pub slips: Vec<Slip>,
    pub rates: DiagnosticRates,
}

#[derive(Clone, Copy, Debug)]
pub struct SlipThresholds {
    pub elevation_mask_deg: f32,
    pub snr_drop_db: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SlipAnalysisConfig {
    pub thresholds: SlipThresholds,
    pub rate_window_min: Option<f32>,
}
