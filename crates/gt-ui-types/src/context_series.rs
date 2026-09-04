use std::sync::Arc;

/// One archived UTC day's interference share, read at the cell the receiver
/// was in nearest that day in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JammingContextSample {
    /// Plot x: the day's UTC midnight as Unix seconds. The share holds for
    /// the whole day.
    pub start_secs: f64,
    /// Share of aircraft reporting low navigation accuracy, in percent.
    /// [`None`] where the archive has no cell over the receiver, or no
    /// recording is loaded to place the day at, which breaks the line.
    pub percent: Option<f64>,
    /// Aircraft the share was computed over. Zero where `percent` is
    /// [`None`].
    pub aircraft: u32,
    /// Aircraft of those that reported low navigation accuracy, for the
    /// hover's counts.
    pub bad: u32,
}

/// One archived geomagnetic index period.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndexContextSample {
    /// Plot x: the period's start as Unix seconds. The value holds for the
    /// index's period length.
    pub start_secs: f64,
    /// [`None`] where the service published no value for the period, which
    /// breaks the line.
    pub value: Option<f64>,
}

/// One archived TEC map epoch, read at the position the receiver was in
/// nearest that epoch in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TecContextSample {
    /// Plot x: the map's epoch as Unix seconds.
    pub x_secs: f64,
    /// Vertical TEC in TEC units. [`None`] where the receiver's position
    /// lies outside the grid, a contributing node is a gap, or no recording
    /// is loaded to place the epoch at, which breaks the line.
    pub tecu: Option<f64>,
}

/// The two geomagnetic index lines, each sampled at its own published
/// cadence.
#[derive(Debug, Clone, Default)]
pub struct GeomagneticContextLines {
    pub hp30: Arc<Vec<IndexContextSample>>,
    pub kp: Arc<Vec<IndexContextSample>>,
}

/// The context metric lines the plot draws across the span it shows, sampled
/// from the archives.
///
/// Each [`Arc`] identity changes exactly when its samples do, which is what
/// the plot rebuilds its mipmaps on.
#[derive(Debug, Clone, Default)]
pub struct ContextLines {
    pub jamming: Arc<Vec<JammingContextSample>>,
    pub geomagnetic: GeomagneticContextLines,
    pub tec: Arc<Vec<TecContextSample>>,
}
