use strum::Display;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Display)]
pub enum SolutionState {
    #[strum(serialize = "Estimated only")]
    EstimatedOnly,
    Fix,
    #[strum(serialize = "No fix")]
    NoFix,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Display)]
pub enum SolutionMethod {
    Autonomous,
    #[strum(serialize = "Dead reckoning")]
    DeadReckoning,
    Differential,
    Hybrid,
    Other,
    #[strum(serialize = "RTK fixed")]
    RtkFixed,
    #[strum(serialize = "RTK float")]
    RtkFloat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Display)]
pub enum SolutionDimension {
    #[strum(serialize = "3D")]
    ThreeD,
    #[strum(serialize = "2D")]
    TwoD,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Display)]
pub enum PositionProvenance {
    #[strum(serialize = "SDK synthesized")]
    SdkSynthesized,
    #[strum(serialize = "Source provided")]
    SourceProvided,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavigationMetadata {
    /// Absence permits fallback. Explicit unknown state blocks fallback.
    pub state: Option<SolutionState>,
    pub method: Option<SolutionMethod>,
    pub dimension: Option<SolutionDimension>,
    pub used_count: Option<u32>,
    pub hdop: Option<f64>,
    pub vdop: Option<f64>,
    pub pdop: Option<f64>,
    pub position_provenance: Option<PositionProvenance>,
}
