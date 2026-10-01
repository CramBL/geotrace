#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnRole {
    Actions { compact_width: f32 },
    Optional { priority: u32 },
    Primary,
    Required,
}

#[derive(Clone, Copy, Debug)]
pub struct ColumnSpec<K> {
    pub key: K,
    pub role: ColumnRole,
    pub minimum_width: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColumnBudget {
    pub available_width: f32,
    pub gap: f32,
    pub pixels_per_point: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionPresentation {
    Compact,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnFallback {
    Fits,
    HorizontalScroll,
}

#[derive(Clone, Copy, Debug)]
pub struct PlannedColumn<K> {
    pub key: K,
    pub width: f32,
}

#[derive(Clone, Debug)]
pub struct ColumnPlan<K> {
    pub columns: Vec<PlannedColumn<K>>,
    pub actions: ActionPresentation,
    pub fallback: ColumnFallback,
    pub width: f32,
}

impl<K: Copy> ColumnPlan<K> {
    pub fn allocate(specs: &[ColumnSpec<K>], budget: ColumnBudget) -> Self {
        assert_eq!(
            specs
                .iter()
                .filter(|spec| spec.role == ColumnRole::Primary)
                .count(),
            1
        );
        assert!(
            specs
                .iter()
                .filter(|spec| matches!(spec.role, ColumnRole::Actions { .. }))
                .count()
                <= 1,
            "column plans support at most one action column"
        );
        let scale = budget.pixels_per_point.max(f32::EPSILON);
        let available = (budget.available_width.max(0.0) * scale).floor() / scale;
        let gap = (budget.gap.max(0.0) * scale).ceil() / scale;
        let rounded_width = |width: f32| (width.max(0.0) * scale).ceil() / scale;
        let mut selected: Vec<bool> = specs
            .iter()
            .map(|spec| !matches!(spec.role, ColumnRole::Optional { .. }))
            .collect();
        let mut widths: Vec<f32> = specs
            .iter()
            .map(|spec| {
                rounded_width(match spec.role {
                    ColumnRole::Actions { compact_width } => compact_width.min(spec.minimum_width),
                    _ => spec.minimum_width,
                })
            })
            .collect();
        let mut occupied = widths
            .iter()
            .zip(&selected)
            .filter(|(_, selected)| **selected)
            .map(|(width, _)| *width)
            .sum::<f32>()
            + gap
                * selected
                    .iter()
                    .filter(|selected| **selected)
                    .count()
                    .saturating_sub(1) as f32;
        let mut optional: Vec<(u32, usize)> = specs
            .iter()
            .enumerate()
            .filter_map(|(index, spec)| match spec.role {
                ColumnRole::Optional { priority } => Some((priority, index)),
                _ => None,
            })
            .collect();
        optional.sort_unstable();
        for (_, index) in optional {
            if let Some(width) = widths.get(index)
                && occupied + gap + width <= available
                && let Some(selected) = selected.get_mut(index)
            {
                *selected = true;
                occupied += gap + width;
            }
        }
        let mut actions = ActionPresentation::Full;
        for (spec, width) in specs.iter().zip(&mut widths) {
            if matches!(spec.role, ColumnRole::Actions { .. }) {
                let full_width = rounded_width(spec.minimum_width);
                if occupied + full_width - *width <= available {
                    occupied += full_width - *width;
                    *width = full_width;
                } else {
                    actions = ActionPresentation::Compact;
                }
            }
        }
        let fallback = if occupied > available {
            ColumnFallback::HorizontalScroll
        } else {
            ColumnFallback::Fits
        };
        if let Some((_, width)) = specs
            .iter()
            .zip(&mut widths)
            .find(|(spec, _)| spec.role == ColumnRole::Primary)
        {
            *width += (available - occupied).max(0.0);
        }
        let columns = specs
            .iter()
            .zip(widths)
            .zip(selected)
            .filter_map(|((spec, width), selected)| {
                selected.then_some(PlannedColumn {
                    key: spec.key,
                    width,
                })
            })
            .collect();
        Self {
            columns,
            actions,
            fallback,
            width: occupied.max(available),
        }
    }
}
