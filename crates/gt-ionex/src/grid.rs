//! The latitude and longitude grid an IONEX header declares its maps on.

use gt_types::{Latitude, Longitude};

/// Degrees in a full turn of longitude, which separates the two conventions
/// a grid is declared in: -180 to 180, or 0 to 360.
const FULL_TURN_DEGREES: f64 = 360.0;

/// How far two declared degree values may differ and still name the same
/// node. Grid records are written to one decimal.
pub const DEGREES_TOLERANCE: f64 = 1e-6;

/// How far outside an axis, in node widths, a query may fall and still count
/// as its edge node.
const EDGE_TOLERANCE_NODES: f64 = 1e-9;

/// Nodes one axis may declare at most, which bounds what a malformed header
/// makes the parser allocate. A global grid at a hundredth of a degree stays
/// under it.
const MAX_NODE_COUNT: usize = 40_000;

/// One axis as a header or map record declares it: `LAT1 / LAT2 / DLAT`,
/// `LON1 / LON2 / DLON`, and the longitude fields of a latitude band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisDeclaration {
    pub first_degrees: f64,
    pub last_degrees: f64,
    pub step_degrees: f64,
}

/// Why a declaration does not describe a grid axis.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum AxisError {
    #[error("the bounds and step must be finite")]
    NotFinite,

    #[error("the step of {step_degrees} deg is below the {DEGREES_TOLERANCE} deg resolution")]
    StepBelowResolution { step_degrees: f64 },

    #[error(
        "the step of {step_degrees} deg runs away from the last node, {first_degrees} deg to {last_degrees} deg"
    )]
    StepAwayFromLastNode {
        first_degrees: f64,
        last_degrees: f64,
        step_degrees: f64,
    },

    #[error(
        "{first_degrees} deg to {last_degrees} deg is not a whole number of {step_degrees} deg steps"
    )]
    PartialStep {
        first_degrees: f64,
        last_degrees: f64,
        step_degrees: f64,
    },

    #[error(
        "{first_degrees} deg to {last_degrees} deg in steps of {step_degrees} deg holds more than {MAX_NODE_COUNT} nodes"
    )]
    TooManyNodes {
        first_degrees: f64,
        last_degrees: f64,
        step_degrees: f64,
    },
}

/// Evenly spaced nodes from [`GridAxis::first_degrees`], one step apart. The
/// step is negative on an axis that descends, which is how latitudes are
/// declared: 87.5 down to -87.5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridAxis {
    first_degrees: f64,
    step_degrees: f64,
    node_count: usize,
}

impl GridAxis {
    pub fn new(declaration: AxisDeclaration) -> Result<Self, AxisError> {
        let AxisDeclaration {
            first_degrees,
            last_degrees,
            step_degrees,
        } = declaration;
        if !first_degrees.is_finite() || !last_degrees.is_finite() || !step_degrees.is_finite() {
            return Err(AxisError::NotFinite);
        }
        if step_degrees.abs() < DEGREES_TOLERANCE {
            return Err(AxisError::StepBelowResolution { step_degrees });
        }

        let span_in_steps = (last_degrees - first_degrees) / step_degrees;
        if span_in_steps < -EDGE_TOLERANCE_NODES {
            return Err(AxisError::StepAwayFromLastNode {
                first_degrees,
                last_degrees,
                step_degrees,
            });
        }
        let steps = span_in_steps.round();
        if (span_in_steps - steps).abs() > DEGREES_TOLERANCE {
            return Err(AxisError::PartialStep {
                first_degrees,
                last_degrees,
                step_degrees,
            });
        }
        let too_many_nodes = AxisError::TooManyNodes {
            first_degrees,
            last_degrees,
            step_degrees,
        };
        let node_count = usize::try_from(steps as i64)
            .ok()
            .and_then(|steps| steps.checked_add(1))
            .filter(|node_count| *node_count <= MAX_NODE_COUNT)
            .ok_or(too_many_nodes)?;

        Ok(Self {
            first_degrees,
            step_degrees,
            node_count,
        })
    }

    pub const fn node_count(self) -> usize {
        self.node_count
    }

    pub const fn first_degrees(self) -> f64 {
        self.first_degrees
    }

    pub const fn step_degrees(self) -> f64 {
        self.step_degrees
    }

    /// The degrees of the node at `index`, or [`None`] past the last node.
    pub fn degrees_at(self, index: usize) -> Option<f64> {
        (index < self.node_count).then_some(self.first_degrees + self.step_degrees * index as f64)
    }

    pub fn last_degrees(self) -> Option<f64> {
        self.degrees_at(self.node_count.checked_sub(1)?)
    }

    /// Whether both axes hold the same nodes, within the rounding of the
    /// degrees a file writes.
    pub fn covers_same_nodes(self, other: Self) -> bool {
        self.node_count == other.node_count
            && (self.first_degrees - other.first_degrees).abs() < DEGREES_TOLERANCE
            && (self.step_degrees - other.step_degrees).abs() < DEGREES_TOLERANCE
    }

    fn position_of_degrees(self, degrees: f64) -> Option<AxisPosition> {
        let last_index = self.node_count.checked_sub(1)?;
        let last_offset = last_index as f64;
        let offset = (degrees - self.first_degrees) / self.step_degrees;
        if offset < -EDGE_TOLERANCE_NODES || offset > last_offset + EDGE_TOLERANCE_NODES {
            return None;
        }

        let offset = offset.clamp(0.0, last_offset);
        let lower_index = usize::try_from(offset.floor() as i64).ok()?.min(last_index);
        Some(AxisPosition {
            lower_index,
            upper_index: lower_index.saturating_add(1).min(last_index),
            fraction: offset - lower_index as f64,
        })
    }
}

/// Where a queried value falls on an axis: between the nodes at
/// `lower_index` and `upper_index`, `fraction` of the way from the first to
/// the second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisPosition {
    pub lower_index: usize,
    pub upper_index: usize,
    pub fraction: f64,
}

impl AxisPosition {
    /// The nearer of the two nodes the position falls between, the upper one
    /// exactly halfway.
    pub fn nearest_index(self) -> usize {
        if self.fraction < 0.5 {
            self.lower_index
        } else {
            self.upper_index
        }
    }
}

/// The grid's latitude axis, declared by `LAT1 / LAT2 / DLAT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatitudeAxis(GridAxis);

impl LatitudeAxis {
    pub const fn new(axis: GridAxis) -> Self {
        Self(axis)
    }

    pub const fn axis(self) -> GridAxis {
        self.0
    }

    pub const fn node_count(self) -> usize {
        self.0.node_count()
    }

    pub fn degrees_at(self, index: usize) -> Option<f64> {
        self.0.degrees_at(index)
    }

    /// Every declared latitude, northernmost first.
    pub fn degrees(self) -> impl Iterator<Item = f64> {
        (0..self.node_count()).filter_map(move |index| self.degrees_at(index))
    }

    /// [`None`] for a latitude outside the declared band.
    pub fn position_of(self, latitude: Latitude) -> Option<AxisPosition> {
        self.0.position_of_degrees(latitude.as_degrees())
    }
}

/// The grid's longitude axis, declared by `LON1 / LON2 / DLON`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongitudeAxis(GridAxis);

impl LongitudeAxis {
    pub const fn new(axis: GridAxis) -> Self {
        Self(axis)
    }

    pub const fn axis(self) -> GridAxis {
        self.0
    }

    pub const fn node_count(self) -> usize {
        self.0.node_count()
    }

    pub fn degrees_at(self, index: usize) -> Option<f64> {
        self.0.degrees_at(index)
    }

    /// A longitude the declared axis does not reach directly is retried a
    /// full turn away, which is what puts a query at 179 deg between the last
    /// two nodes of a 0 to 360 deg grid.
    pub fn position_of(self, longitude: Longitude) -> Option<AxisPosition> {
        let degrees = longitude.as_degrees();
        [
            degrees,
            degrees + FULL_TURN_DEGREES,
            degrees - FULL_TURN_DEGREES,
        ]
        .into_iter()
        .find_map(|candidate| self.0.position_of_degrees(candidate))
    }
}

/// The grid every map in one file is published on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapGrid {
    pub latitudes: LatitudeAxis,
    pub longitudes: LongitudeAxis,
    /// Height of the single shell the maps model the ionosphere as, from
    /// `HGT1 / HGT2 / DHGT`.
    pub shell_height_km: f64,
}

impl MapGrid {
    /// The node nearest a position, or [`None`] where the position lies off
    /// the grid.
    pub fn nearest_node(self, latitude: Latitude, longitude: Longitude) -> Option<GridPoint> {
        Some(GridPoint {
            latitude_index: self.latitudes.position_of(latitude)?.nearest_index(),
            longitude_index: self.longitudes.position_of(longitude)?.nearest_index(),
        })
    }
}

/// One node of a [`MapGrid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPoint {
    pub latitude_index: usize,
    pub longitude_index: usize,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// The axes JPL and CODE publish global maps on.
    fn jpl_latitudes() -> LatitudeAxis {
        LatitudeAxis::new(
            GridAxis::new(AxisDeclaration {
                first_degrees: 87.5,
                last_degrees: -87.5,
                step_degrees: -2.5,
            })
            .unwrap(),
        )
    }

    fn jpl_longitudes() -> LongitudeAxis {
        LongitudeAxis::new(
            GridAxis::new(AxisDeclaration {
                first_degrees: -180.0,
                last_degrees: 180.0,
                step_degrees: 5.0,
            })
            .unwrap(),
        )
    }

    #[test]
    fn the_published_axes_hold_the_nodes_their_bounds_and_step_name() {
        assert_eq!(jpl_latitudes().node_count(), 71);
        assert_eq!(jpl_latitudes().degrees_at(0), Some(87.5));
        assert_eq!(jpl_latitudes().degrees_at(70), Some(-87.5));
        assert_eq!(jpl_latitudes().degrees_at(71), None);
        assert_eq!(jpl_longitudes().node_count(), 73);
        assert_eq!(jpl_longitudes().degrees_at(72), Some(180.0));
        assert_eq!(jpl_longitudes().axis().last_degrees(), Some(180.0));
    }

    #[rstest]
    #[case::the_first_node(87.5, 0, 1, 0.0)]
    #[case::the_last_node(-87.5, 70, 70, 0.0)]
    #[case::a_node_in_between(0.0, 35, 36, 0.0)]
    #[case::halfway_between_two_nodes(1.25, 34, 35, 0.5)]
    #[case::three_fifths_past_a_node(86.0, 0, 1, 0.6)]
    fn a_latitude_falls_between_its_two_neighbouring_nodes(
        #[case] degrees: f64,
        #[case] lower_index: usize,
        #[case] upper_index: usize,
        #[case] fraction: f64,
    ) {
        let position = jpl_latitudes().position_of(Latitude::new(degrees)).unwrap();
        assert_eq!(position.lower_index, lower_index);
        assert_eq!(position.upper_index, upper_index);
        assert!(
            (position.fraction - fraction).abs() < 1e-9,
            "{position:?} is not {fraction} past node {lower_index}"
        );
    }

    /// A position between two nodes takes the nearer of them, and the upper
    /// node exactly halfway.
    #[rstest]
    #[case::just_short_of_halfway(86.26, 0)]
    #[case::halfway(86.25, 1)]
    #[case::just_past_halfway(86.24, 1)]
    fn a_position_reads_back_as_the_node_nearest_it(#[case] degrees: f64, #[case] expected: usize) {
        let position = jpl_latitudes()
            .position_of(Latitude::new(degrees))
            .expect("a latitude on the grid");

        assert_eq!(position.nearest_index(), expected);
    }

    /// Both axes round together, and a position off the grid names no node
    /// at all.
    #[rstest]
    #[case::a_node_itself(87.5, -180.0, Some((0, 0)))]
    #[case::the_nearest_corner_of_a_cell(86.0, -178.0, Some((1, 0)))]
    #[case::north_of_the_grid(89.0, 0.0, None)]
    fn a_grid_reads_back_the_node_nearest_a_position(
        #[case] latitude: f64,
        #[case] longitude: f64,
        #[case] expected: Option<(usize, usize)>,
    ) {
        let grid = MapGrid {
            latitudes: jpl_latitudes(),
            longitudes: jpl_longitudes(),
            shell_height_km: 450.0,
        };

        assert_eq!(
            grid.nearest_node(Latitude::new(latitude), Longitude::new(longitude)),
            expected.map(|(latitude_index, longitude_index)| GridPoint {
                latitude_index,
                longitude_index,
            })
        );
    }

    #[rstest]
    #[case::north_of_the_grid(89.0)]
    #[case::south_of_the_grid(-89.0)]
    #[case::the_north_pole(90.0)]
    fn a_latitude_outside_the_grid_has_no_position(#[case] degrees: f64) {
        assert_eq!(jpl_latitudes().position_of(Latitude::new(degrees)), None);
    }

    /// Both ends of a global grid name the same meridian, so a query there
    /// lands on a node either way round.
    #[test]
    fn the_repeated_meridian_is_reached_from_both_ends() {
        let axis = jpl_longitudes();
        assert_eq!(
            axis.position_of(Longitude::new(-180.0)),
            Some(AxisPosition {
                lower_index: 0,
                upper_index: 1,
                fraction: 0.0
            })
        );
        assert_eq!(
            axis.position_of(Longitude::new(180.0)),
            Some(AxisPosition {
                lower_index: 72,
                upper_index: 72,
                fraction: 0.0
            })
        );
    }

    /// A grid declared in the 0 to 360 convention covers a negative longitude
    /// a full turn away.
    #[test]
    fn an_eastward_grid_covers_a_western_longitude() {
        let axis = LongitudeAxis::new(
            GridAxis::new(AxisDeclaration {
                first_degrees: 0.0,
                last_degrees: 355.0,
                step_degrees: 5.0,
            })
            .unwrap(),
        );
        assert_eq!(axis.node_count(), 72);
        assert_eq!(
            axis.position_of(Longitude::new(-5.0)),
            Some(AxisPosition {
                lower_index: 71,
                upper_index: 71,
                fraction: 0.0
            })
        );
        assert_eq!(
            axis.position_of(Longitude::new(-180.0)),
            Some(AxisPosition {
                lower_index: 36,
                upper_index: 37,
                fraction: 0.0
            })
        );
    }

    /// A regional grid reaches neither the wrapped longitude nor anything
    /// outside its bounds.
    #[test]
    fn a_regional_grid_covers_only_its_own_longitudes() {
        let axis = LongitudeAxis::new(
            GridAxis::new(AxisDeclaration {
                first_degrees: 0.0,
                last_degrees: 30.0,
                step_degrees: 5.0,
            })
            .unwrap(),
        );
        assert!(axis.position_of(Longitude::new(15.0)).is_some());
        assert_eq!(axis.position_of(Longitude::new(-15.0)), None);
        assert_eq!(axis.position_of(Longitude::new(45.0)), None);
    }

    #[test]
    fn a_single_node_axis_covers_only_that_node() {
        let axis = GridAxis::new(AxisDeclaration {
            first_degrees: 10.0,
            last_degrees: 10.0,
            step_degrees: 2.5,
        })
        .unwrap();
        assert_eq!(axis.node_count(), 1);
        assert_eq!(
            LatitudeAxis::new(axis).position_of(Latitude::new(10.0)),
            Some(AxisPosition {
                lower_index: 0,
                upper_index: 0,
                fraction: 0.0
            })
        );
        assert_eq!(
            LatitudeAxis::new(axis).position_of(Latitude::new(12.5)),
            None
        );
    }

    #[rstest]
    #[case::a_zero_step(0.0, 90.0, 0.0, AxisError::StepBelowResolution { step_degrees: 0.0 })]
    #[case::a_step_towards_the_wrong_end(
        87.5,
        -87.5,
        2.5,
        AxisError::StepAwayFromLastNode { first_degrees: 87.5, last_degrees: -87.5, step_degrees: 2.5 }
    )]
    #[case::a_span_the_step_does_not_divide(
        0.0,
        10.0,
        3.0,
        AxisError::PartialStep { first_degrees: 0.0, last_degrees: 10.0, step_degrees: 3.0 }
    )]
    #[case::more_nodes_than_a_grid_holds(
        0.0,
        360.0,
        0.000_1,
        AxisError::TooManyNodes { first_degrees: 0.0, last_degrees: 360.0, step_degrees: 0.000_1 }
    )]
    #[case::an_infinite_bound(0.0, f64::INFINITY, 2.5, AxisError::NotFinite)]
    #[case::a_bound_that_is_not_a_number(f64::NAN, 90.0, 2.5, AxisError::NotFinite)]
    fn a_declaration_that_is_not_a_grid_names_what_is_wrong(
        #[case] first_degrees: f64,
        #[case] last_degrees: f64,
        #[case] step_degrees: f64,
        #[case] expected: AxisError,
    ) {
        assert_eq!(
            GridAxis::new(AxisDeclaration {
                first_degrees,
                last_degrees,
                step_degrees
            }),
            Err(expected)
        );
    }

    #[test]
    fn axes_written_to_different_decimals_cover_the_same_nodes() {
        let declared = GridAxis::new(AxisDeclaration {
            first_degrees: -180.0,
            last_degrees: 180.0,
            step_degrees: 5.0,
        })
        .unwrap();
        let rounded = GridAxis::new(AxisDeclaration {
            first_degrees: -180.000_000_1,
            last_degrees: 180.0,
            step_degrees: 5.0,
        })
        .unwrap();
        assert!(declared.covers_same_nodes(rounded));
        assert!(
            !declared.covers_same_nodes(
                GridAxis::new(AxisDeclaration {
                    first_degrees: -180.0,
                    last_degrees: 180.0,
                    step_degrees: 2.5,
                })
                .unwrap()
            )
        );
    }
}
