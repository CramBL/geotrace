#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SolutionState {
    EstimatedOnly,
    Fix,
    NoFix,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SolutionMethod {
    Autonomous,
    DeadReckoning,
    Differential,
    Hybrid,
    Other,
    RtkFixed,
    RtkFloat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SolutionDimension {
    ThreeD,
    TwoD,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PositionProvenance {
    SdkSynthesized,
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
