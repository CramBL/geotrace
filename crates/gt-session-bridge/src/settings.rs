use serde::{Deserialize, Serialize};
use strum::EnumIter;

#[derive(Clone, Copy, Debug, Deserialize, EnumIter, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SettingKey {
    #[serde(rename = "analysis.clock_excursion_threshold_s")]
    AnalysisClockExcursionThreshold,
    #[serde(rename = "analysis.elevation_mask_deg")]
    AnalysisElevationMask,
    #[serde(rename = "analysis.mark_backward_time_steps")]
    AnalysisMarkBackwardTimeSteps,
    #[serde(rename = "analysis.mark_masked_fix")]
    AnalysisMarkMaskedFix,
    #[serde(rename = "analysis.slip_window_min")]
    AnalysisSlipWindow,
    #[serde(rename = "analysis.snr_drop_db")]
    AnalysisSnrDropThreshold,
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
    Live,
    LiveWithDeferredMarkers,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingUnit {
    DecibelHertz,
    Degrees,
    Minutes,
    Seconds,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SettingValue {
    Boolean {
        value: bool,
    },
    Number {
        value: Option<f32>,
        minimum: f32,
        maximum: f32,
        unit: SettingUnit,
    },
}
