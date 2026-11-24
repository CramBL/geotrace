use gt_query::{ChannelConflict, ChannelInfo, ConstructKind, MetricSource, QueryMetric};
use gt_types::LoadedFile;
use serde::{Deserialize, Serialize};

use crate::framing::Frame;
use crate::protocol::ListMetrics;

const MAX_CHANNELS_PER_PAGE: usize = 100;
const MAX_CONFLICTS_PER_CHANNEL: usize = 8;
// The 256 KiB reply includes 100 entries at most, plus the built-in definitions.
const MAX_CHANNEL_BYTES: usize = 2 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Metrics {
    pub definitions: Vec<MetricDefinition>,
    pub channels: Vec<ChannelEntry>,
    pub next_channel_start: Option<usize>,
}

impl Metrics {
    pub(crate) fn from_files(files: &[LoadedFile], args: ListMetrics) -> Self {
        let definitions = gt_query::catalog()
            .iter()
            .filter(|construct| construct.kind == ConstructKind::Metric)
            .filter_map(|construct| {
                let metric: QueryMetric = construct.name.parse().ok()?;
                let quantity = metric.quantity();
                Some(MetricDefinition {
                    name: construct.name.to_owned(),
                    summary: construct.summary.to_owned(),
                    quantity: quantity.to_string(),
                    units: quantity.canonical_units().map(str::to_owned).collect(),
                    source: metric.source(),
                })
            })
            .collect();
        let schema = gt_query_run::schema_from_files(files);
        let mut channels: Vec<_> = schema.iter_unsorted().collect();
        channels.sort_unstable_by_key(|&(name, _)| name);
        let start = args.channel_start.unwrap_or_default();
        let limit = args.channel_limit.map_or(MAX_CHANNELS_PER_PAGE, |limit| {
            limit.get().min(MAX_CHANNELS_PER_PAGE)
        });
        let end = start.saturating_add(limit);
        let next_channel_start = (end < channels.len()).then_some(end);
        let channels = channels
            .into_iter()
            .skip(start)
            .take(limit)
            .map(|(name, info)| ChannelEntry::new(name, info))
            .collect();
        Self {
            definitions,
            channels,
            next_channel_start,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MetricDefinition {
    pub name: String,
    pub summary: String,
    pub quantity: String,
    pub units: Vec<String>,
    pub source: MetricSource,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChannelEntry {
    Defined(ChannelDefinition),
    MetadataOmitted { name: Option<String> },
}

impl ChannelEntry {
    fn new(name: &str, info: &ChannelInfo) -> Self {
        if let Some(definition) = ChannelDefinition::new(name, info) {
            let entry = Self::Defined(definition);
            if Frame::encode(&entry, MAX_CHANNEL_BYTES).is_ok() {
                return entry;
            }
        }
        let entry = Self::MetadataOmitted {
            name: (name.len() <= MAX_CHANNEL_BYTES).then(|| name.to_owned()),
        };
        if Frame::encode(&entry, MAX_CHANNEL_BYTES).is_ok() {
            entry
        } else {
            Self::MetadataOmitted { name: None }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChannelDefinition {
    pub name: String,
    pub reference: Option<String>,
    pub unit: Option<String>,
    pub period_deg: Option<f64>,
    pub components: Vec<ChannelComponent>,
    pub conflicts: Vec<String>,
    pub conflict_count: usize,
}

impl ChannelDefinition {
    fn new(name: &str, info: &ChannelInfo) -> Option<Self> {
        let unit = info.unit.as_ref().map(|unit| unit.label());
        let remaining = MAX_CHANNEL_BYTES
            .checked_sub(name.len())?
            .checked_sub(unit.map_or(0, str::len))?;
        info.components
            .iter()
            .try_fold(remaining, |remaining, label| {
                remaining.checked_sub(label.len().max(1))
            })?;
        Some(Self {
            name: name.to_owned(),
            reference: gt_query::channel_reference(name, None),
            unit: unit.map(str::to_owned),
            period_deg: info.period_deg,
            components: info
                .components
                .iter()
                .enumerate()
                .map(|(index, label)| ChannelComponent {
                    label: label.clone(),
                    reference: if info.component_index(label) == Some(index) {
                        gt_query::channel_reference(name, Some(label))
                    } else {
                        None
                    },
                })
                .collect(),
            conflicts: info
                .conflicts
                .iter()
                .take(MAX_CONFLICTS_PER_CHANNEL)
                .map(ChannelConflict::description)
                .collect(),
            conflict_count: info.conflicts.len(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ChannelComponent {
    pub label: String,
    pub reference: Option<String>,
}
