//! Strategies for synthetic datasets and query programs, and the renderer that
//! turns a program into editor text.
//!
//! A program is generated as data and rendered afterwards, so the same program
//! can be written several ways and the pictures compared.

use gt_query_map_harness::{Dataset, FileSpec, MapScenario, PointSpec, TrackSpec};
use proptest::prelude::*;

/// Metrics the synthetic points carry, and a query may therefore read.
///
/// `heading` is a direction, and its aggregates are circular statistics. The
/// oracle implements no circular statistics, so heading appears in per-point
/// predicates only (see [`agg_metric`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Velocity,
    Eph,
    Heading,
    Accel,
}

impl Metric {
    pub fn name(self) -> &'static str {
        match self {
            Self::Velocity => "velocity",
            Self::Eph => "eph",
            Self::Heading => "heading",
            Self::Accel => "accel",
        }
    }

    /// The unit a threshold on this metric is written in.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Velocity => "km/h",
            Self::Eph => "m",
            Self::Heading => "deg",
            Self::Accel => "m/s2",
        }
    }

    /// Thresholds in this metric's own unit, spanning the values the data
    /// carries so a predicate splits the points rather than matching all or
    /// none.
    fn threshold(self) -> BoxedStrategy<f64> {
        match self {
            Self::Velocity => (0i32..=45).prop_map(f64::from).boxed(),
            Self::Eph => (0i32..=30).prop_map(f64::from).boxed(),
            Self::Heading => (0i32..=36).prop_map(|d| f64::from(d) * 10.0).boxed(),
            Self::Accel => (-6i32..=6).prop_map(|a| f64::from(a) * 0.5).boxed(),
        }
    }
}

/// Aggregates over a window. All are linear reductions, so the oracle needs no
/// circular statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agg {
    Avg,
    Min,
    Max,
    Spread,
}

impl Agg {
    pub fn name(self) -> &'static str {
        match self {
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
            Self::Spread => "spread",
        }
    }
}

/// What a comparison's left side reads: a metric at the point, or an aggregate
/// over the window. The checker rejects a bare metric under a window and an
/// aggregate without one, so a stage's window determines which of these it holds.
#[derive(Debug, Clone, Copy)]
pub enum Term {
    Point(Metric),
    Agg { func: Agg, metric: Metric },
}

impl Term {
    pub fn metric(self) -> Metric {
        match self {
            Self::Point(metric) | Self::Agg { metric, .. } => metric,
        }
    }

    fn render(self) -> String {
        match self {
            Self::Point(metric) => metric.name().to_owned(),
            Self::Agg { func, metric } => format!("{}({})", func.name(), metric.name()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn render(self) -> &'static str {
        match self {
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }
}

/// A generated predicate tree.
#[derive(Debug, Clone)]
pub enum Predicate {
    Cmp {
        term: Term,
        op: CmpOp,
        threshold: f64,
    },
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    /// Every metric this predicate reads.
    pub fn metrics(&self) -> Vec<Metric> {
        match self {
            Self::Cmp { term, .. } => vec![term.metric()],
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => {
                let mut metrics = lhs.metrics();
                metrics.extend(rhs.metrics());
                metrics
            }
            Self::Not(inner) => inner.metrics(),
        }
    }

    /// Written fully parenthesized, so rendering never has to reason about
    /// precedence.
    fn render(&self) -> String {
        match self {
            Self::Cmp {
                term,
                op,
                threshold,
            } => format!(
                "{} {} {threshold} {}",
                term.render(),
                op.render(),
                term.metric().unit()
            ),
            Self::And(lhs, rhs) => format!("({}) and ({})", lhs.render(), rhs.render()),
            Self::Or(lhs, rhs) => format!("({}) or ({})", lhs.render(), rhs.render()),
            Self::Not(inner) => format!("not ({})", inner.render()),
        }
    }
}

/// What a stage does with the points it matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Keep,
    Hide,
    Draw,
}

impl Mode {
    fn render(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Hide => "hide",
            Self::Draw => "draw",
        }
    }
}

/// One stage: an optional window, a predicate, and what to do with the match.
#[derive(Debug, Clone)]
pub struct Stage {
    pub mode: Mode,
    pub window: Option<usize>,
    pub predicate: Predicate,
}

impl Stage {
    /// Whether this stage's verdict at a point depends on that point alone.
    ///
    /// A window reaches across neighbours, and `accel` differences against the
    /// previous point *of the current run* - either makes the verdict depend on
    /// which other points are still visible, so the stage stops commuting with
    /// its neighbours.
    pub fn is_point_local(&self) -> bool {
        self.window.is_none() && !self.predicate.metrics().contains(&Metric::Accel)
    }

    /// The stage as one query, its pipe-separated parts already in order.
    fn parts(&self) -> Vec<String> {
        let mut parts = vec!["points".to_owned()];
        if let Some(window) = self.window {
            parts.push(format!("| window {window}"));
        }
        parts.push(format!("| where {}", self.predicate.render()));
        parts.push(format!("| {}", self.mode.render()));
        parts
    }
}

/// A generated query program: the stages of one pipeline, in editor order.
#[derive(Debug, Clone)]
pub struct Program {
    pub stages: Vec<Stage>,
}

impl Program {
    /// The first `count` stages.
    pub fn prefix(&self, count: usize) -> Self {
        Self {
            stages: self.stages.iter().take(count).cloned().collect(),
        }
    }

    /// The program with stage `index` written twice in a row.
    pub fn with_stage_repeated(&self, index: usize) -> Self {
        let mut stages = self.stages.clone();
        if let Some(stage) = stages.get(index).cloned() {
            stages.insert(index, stage);
        }
        Self { stages }
    }

    /// The program with stages `index` and `index + 1` exchanged.
    pub fn with_adjacent_swapped(&self, index: usize) -> Self {
        let mut stages = self.stages.clone();
        if index + 1 < stages.len() {
            stages.swap(index, index + 1);
        }
        Self { stages }
    }

    /// Which draw layer each `draw` stage becomes, by stage index.
    pub fn draw_layers(&self) -> Vec<usize> {
        self.stages
            .iter()
            .enumerate()
            .filter(|(_, stage)| stage.mode == Mode::Draw)
            .map(|(index, _)| index)
            .collect()
    }

    /// The program as editor text in the plainest style: one blank line between
    /// stages, LF endings, no wrapping.
    pub fn render_plain(&self) -> String {
        self.render(&RenderStyle::plain())
    }

    /// The program as editor text, written in `style`.
    pub fn render(&self, style: &RenderStyle) -> String {
        let nl = style.newline.render();
        let mut text = String::new();
        for _ in 0..style.leading_blanks {
            text.push_str(nl);
        }
        for (index, stage) in self.stages.iter().enumerate() {
            if index > 0 {
                text.push_str(&style.separator(index - 1).render(nl));
            }
            text.push_str(&style.wrap.render(&stage.parts(), nl));
        }
        for _ in 0..style.trailing_blanks {
            text.push_str(nl);
        }
        text
    }
}

/// Line endings the editor buffer may hold - a pasted query brings whatever its
/// source used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    Lf,
    Crlf,
}

impl Newline {
    fn render(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }
}

/// How one gap between two queries is written. Every variant is a blank line by
/// the editor's reckoning, so all of them separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    OneBlank,
    TwoBlanks,
    ThreeBlanks,
    SpacesOnly,
    TabsOnly,
    SpacesAndBlanks,
}

impl Separator {
    fn render(self, nl: &str) -> String {
        let blanks: &[&str] = match self {
            Self::OneBlank => &[""],
            Self::TwoBlanks => &["", ""],
            Self::ThreeBlanks => &["", "", ""],
            Self::SpacesOnly => &["   "],
            Self::TabsOnly => &["\t"],
            Self::SpacesAndBlanks => &["", " \t", ""],
        };
        // The line the stage ended on has to be closed first. Each entry after
        // that contributes one blank line.
        let mut text = nl.to_owned();
        for blank in blanks {
            text.push_str(blank);
            text.push_str(nl);
        }
        text
    }
}

/// Whether a query is written on one line or wrapped at its pipes, and how far
/// the continuation lines are indented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    Inline,
    Wrapped { indent: usize },
}

impl Wrap {
    fn render(self, parts: &[String], nl: &str) -> String {
        match self {
            Self::Inline => parts.join(" "),
            Self::Wrapped { indent } => {
                let pad = " ".repeat(indent);
                parts.join(&format!("{nl}{pad}"))
            }
        }
    }
}

/// How a program is written out: the messiness a real editor buffer collects.
#[derive(Debug, Clone)]
pub struct RenderStyle {
    pub newline: Newline,
    /// One entry per gap between stages, reused cyclically when the program has
    /// more gaps than entries.
    pub separators: Vec<Separator>,
    pub leading_blanks: usize,
    pub trailing_blanks: usize,
    pub wrap: Wrap,
}

impl RenderStyle {
    /// The plainest style there is, the baseline the others are compared to.
    pub fn plain() -> Self {
        Self {
            newline: Newline::Lf,
            separators: vec![Separator::OneBlank],
            leading_blanks: 0,
            trailing_blanks: 0,
            wrap: Wrap::Inline,
        }
    }

    fn separator(&self, gap: usize) -> Separator {
        self.separators
            .get(gap % self.separators.len().max(1))
            .copied()
            .unwrap_or(Separator::OneBlank)
    }
}

/// One synthetic point: how long after the previous one it was recorded, and
/// which metrics it carries.
#[derive(Debug, Clone, Copy)]
pub struct GenPoint {
    pub gap_secs: i64,
    pub speed_kmh: Option<f64>,
    pub heading_deg: Option<f64>,
    pub eph_m: Option<f32>,
}

/// A generated dataset: files of tracks, and the time window the scenario
/// filters by.
#[derive(Debug, Clone)]
pub struct GenDataset {
    pub files: Vec<Vec<Vec<GenPoint>>>,
    /// Inclusive window in seconds from the dataset epoch, or no filter.
    pub window_secs: Option<(i64, i64)>,
}

impl GenDataset {
    /// The harness dataset these points describe.
    pub fn dataset(&self) -> Dataset {
        let specs: Vec<FileSpec> = self
            .files
            .iter()
            .enumerate()
            .map(|(index, tracks)| {
                FileSpec::with_tracks(
                    &format!("gen{index}.gtd"),
                    tracks.iter().map(|points| track_spec(points)).collect(),
                )
            })
            .collect();
        Dataset::of_files(&specs)
    }

    /// A scenario over this dataset with its time filter already applied.
    pub fn scenario(&self) -> MapScenario {
        let mut scenario = MapScenario::new(self.dataset());
        if let Some((start, end)) = self.window_secs {
            scenario.set_time_filter_secs(Some(start), Some(end));
        }
        scenario
    }

    /// The longest track, which bounds the windows a program can usefully
    /// request.
    pub fn longest_track(&self) -> usize {
        self.files
            .iter()
            .flatten()
            .map(|points| points.len())
            .max()
            .unwrap_or(0)
    }
}

fn track_spec(points: &[GenPoint]) -> TrackSpec {
    let mut secs = 0;
    TrackSpec::from_points(
        points
            .iter()
            .map(|point| {
                secs += point.gap_secs;
                let mut spec = PointSpec::at_secs(secs);
                spec.speed_kmh = point.speed_kmh;
                spec.heading_deg = point.heading_deg;
                spec.eph_m = point.eph_m;
                spec
            })
            .collect(),
    )
}

/// A point whose metrics are present often but not always, so predicates meet
/// missing values.
fn gen_point() -> impl Strategy<Value = GenPoint> {
    (
        1i64..=20,
        prop_oneof![
            9 => (0i32..=40).prop_map(|kmh| Some(f64::from(kmh))),
            1 => Just(None),
        ],
        prop_oneof![
            8 => (0i32..=35).prop_map(|d| Some(f64::from(d) * 10.0)),
            2 => Just(None),
        ],
        prop_oneof![
            8 => (0i32..=25).prop_map(|m| Some(m as f32)),
            2 => Just(None),
        ],
    )
        .prop_map(|(gap_secs, speed_kmh, heading_deg, eph_m)| GenPoint {
            gap_secs,
            speed_kmh,
            heading_deg,
            eph_m,
        })
}

/// A dataset of one or two files of one or two tracks, tracks of one to ten
/// points, and a time window half the time.
///
/// Tracks of one file sit a recording apart, so a window that slices the first
/// drops the second entirely - both a slice and a whole-track exclusion in one
/// dataset. Tracks in different files start together, so a window slices both.
pub fn gen_dataset() -> impl Strategy<Value = GenDataset> {
    let tracks = proptest::collection::vec(proptest::collection::vec(gen_point(), 1..=10), 1..=2);
    let files = proptest::collection::vec(tracks, 1..=2);
    let window = prop_oneof![
        1 => Just(None),
        1 => (0i64..=40, 1i64..=60).prop_map(|(start, span)| Some((start, start + span))),
    ];
    (files, window).prop_map(|(files, window_secs)| GenDataset { files, window_secs })
}

fn cmp_op() -> impl Strategy<Value = CmpOp> {
    prop_oneof![
        Just(CmpOp::Lt),
        Just(CmpOp::Le),
        Just(CmpOp::Gt),
        Just(CmpOp::Ge),
    ]
}

fn point_metric() -> impl Strategy<Value = Metric> {
    prop_oneof![
        Just(Metric::Velocity),
        Just(Metric::Eph),
        Just(Metric::Heading),
        Just(Metric::Accel),
    ]
}

/// Metrics an aggregate may reduce: the linear ones, so the oracle reduces them
/// with plain arithmetic.
fn agg_metric() -> impl Strategy<Value = Metric> {
    prop_oneof![Just(Metric::Velocity), Just(Metric::Eph)]
}

fn agg_func() -> impl Strategy<Value = Agg> {
    prop_oneof![
        Just(Agg::Avg),
        Just(Agg::Min),
        Just(Agg::Max),
        Just(Agg::Spread),
    ]
}

fn comparison(term: impl Strategy<Value = Term>) -> impl Strategy<Value = Predicate> {
    term.prop_flat_map(|term| {
        (cmp_op(), term.metric().threshold()).prop_map(move |(op, threshold)| Predicate::Cmp {
            term,
            op,
            threshold,
        })
    })
}

/// A predicate tree over `term`, up to a couple of connectives deep.
fn predicate(term: BoxedStrategy<Term>) -> impl Strategy<Value = Predicate> {
    comparison(term).prop_recursive(2, 6, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone())
                .prop_map(|(lhs, rhs)| Predicate::And(Box::new(lhs), Box::new(rhs))),
            (inner.clone(), inner.clone())
                .prop_map(|(lhs, rhs)| Predicate::Or(Box::new(lhs), Box::new(rhs))),
            inner.prop_map(|inner| Predicate::Not(Box::new(inner))),
        ]
    })
}

fn mode() -> impl Strategy<Value = Mode> {
    prop_oneof![Just(Mode::Keep), Just(Mode::Hide), Just(Mode::Draw)]
}

/// One stage. `max_window` bounds the window size a stage may request, so a
/// generated program stays relevant to the dataset it runs over - a window
/// longer than every track would make every stage vacuous.
fn gen_stage(max_window: usize) -> impl Strategy<Value = Stage> {
    let windowed = (2..=max_window.max(2))
        .prop_flat_map(|window| {
            let term = (agg_func(), agg_metric())
                .prop_map(|(func, metric)| Term::Agg { func, metric })
                .boxed();
            (Just(window), predicate(term))
        })
        .prop_map(|(window, predicate)| (Some(window), predicate));
    let plain = predicate(point_metric().prop_map(Term::Point).boxed())
        .prop_map(|predicate| (None, predicate));
    (mode(), prop_oneof![2 => plain, 1 => windowed]).prop_map(|(mode, (window, predicate))| Stage {
        mode,
        window,
        predicate,
    })
}

/// One to four stages, windowed relative to `max_window`.
pub fn gen_program(max_window: usize) -> impl Strategy<Value = Program> {
    proptest::collection::vec(gen_stage(max_window), 1..=4).prop_map(|stages| Program { stages })
}

/// A program every stage of which judges a point by that point alone: no
/// window, and no `accel` to difference against a neighbour.
pub fn gen_point_local_program() -> impl Strategy<Value = Program> {
    let term = prop_oneof![
        Just(Metric::Velocity),
        Just(Metric::Eph),
        Just(Metric::Heading),
    ]
    .prop_map(Term::Point)
    .boxed();
    let stage = (mode(), predicate(term)).prop_map(|(mode, predicate)| Stage {
        mode,
        window: None,
        predicate,
    });
    proptest::collection::vec(stage, 1..=4).prop_map(|stages| Program { stages })
}

/// A dataset and a program whose windows suit it.
pub fn gen_dataset_and_program() -> impl Strategy<Value = (GenDataset, Program)> {
    dataset_and_program(gen_dataset())
}

/// Datasets that always carry a time filter, for the properties about slicing.
pub fn gen_windowed_dataset() -> impl Strategy<Value = GenDataset> {
    gen_dataset().prop_map(|dataset| GenDataset {
        window_secs: dataset.window_secs.or(Some((0, 45))),
        ..dataset
    })
}

/// The same, paired with a program whose windows suit it.
pub fn gen_windowed_dataset_and_program() -> impl Strategy<Value = (GenDataset, Program)> {
    dataset_and_program(gen_windowed_dataset())
}

fn dataset_and_program(
    datasets: impl Strategy<Value = GenDataset>,
) -> impl Strategy<Value = (GenDataset, Program)> {
    datasets.prop_flat_map(|dataset| {
        // One past the longest track too, so a window no run can fill is
        // exercised as well.
        let max_window = dataset.longest_track() + 1;
        (Just(dataset), gen_program(max_window))
    })
}

/// A rendering style, every axis of messiness varied.
pub fn gen_render_style() -> impl Strategy<Value = RenderStyle> {
    (
        prop_oneof![Just(Newline::Lf), Just(Newline::Crlf)],
        proptest::collection::vec(
            prop_oneof![
                Just(Separator::OneBlank),
                Just(Separator::TwoBlanks),
                Just(Separator::ThreeBlanks),
                Just(Separator::SpacesOnly),
                Just(Separator::TabsOnly),
                Just(Separator::SpacesAndBlanks),
            ],
            1..=4,
        ),
        0usize..=2,
        0usize..=2,
        prop_oneof![
            Just(Wrap::Inline),
            (0usize..=4).prop_map(|indent| Wrap::Wrapped { indent }),
        ],
    )
        .prop_map(
            |(newline, separators, leading_blanks, trailing_blanks, wrap)| RenderStyle {
                newline,
                separators,
                leading_blanks,
                trailing_blanks,
                wrap,
            },
        )
}
