use serde::{Deserialize, Serialize};
use strum::EnumIter;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum SettingKey {
    Analysis(AnalysisSettingKey),
    Processing(ProcessingSettingKey),
}

#[derive(Clone, Copy, Debug, Deserialize, EnumIter, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AnalysisSettingKey {
    #[serde(rename = "analysis.clock_excursion_threshold_s")]
    ClockExcursionThreshold,
    #[serde(rename = "analysis.elevation_mask_deg")]
    ElevationMask,
    #[serde(rename = "analysis.mark_backward_time_steps")]
    MarkBackwardTimeSteps,
    #[serde(rename = "analysis.mark_masked_fix")]
    MarkMaskedFix,
    #[serde(rename = "analysis.slip_window_min")]
    SlipWindow,
    #[serde(rename = "analysis.snr_drop_db")]
    SnrDropThreshold,
}

#[derive(Clone, Copy, Debug, Deserialize, EnumIter, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ProcessingSettingKey {
    #[serde(rename = "processing.clock_discontinuity_sigmas")]
    ClockDiscontinuitySigmas,
    #[serde(rename = "processing.detect_clock_discontinuities")]
    DetectClockDiscontinuities,
    #[serde(rename = "processing.detect_clock_offset_excursions")]
    DetectClockOffsetExcursions,
    #[serde(rename = "processing.detect_gnss_fix_lost")]
    DetectGnssFixLost,
    #[serde(rename = "processing.detect_gnss_fix_regained")]
    DetectGnssFixRegained,
    #[serde(rename = "processing.detect_slips")]
    DetectSlips,
    #[serde(rename = "processing.track_split_gap_seconds")]
    TrackSplitGap,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SettingSelection {
    pub key: SettingKey,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingInfo {
    pub key: SettingKey,
    pub page: String,
    pub label: String,
    pub help: String,
    pub apply: SettingApply,
    #[serde(flatten)]
    pub value: SettingValue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingApply {
    DeferredProcessing,
    Live,
    LiveWithDeferredMarkers,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingUnit {
    DecibelHertz,
    Degrees,
    Minutes,
    RobustStandardDeviations,
    Seconds,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SettingValue {
    Boolean {
        value: bool,
    },
    Number {
        value: Option<f64>,
        minimum: f64,
        maximum: f64,
        unit: SettingUnit,
    },
}
