use geotrace_sdk_units::ChannelUnit;
use gt_query::{ChannelConflict, ChannelInfo, ChannelSchema};
use gt_types::LoadedFile;
use uom::si::angle::degree;

/// The schema the editor checks against: every scalar or vector channel across
/// the loaded files, keyed by name. A channel is queryable if any loaded track
/// carries it; a run over a track lacking it reports the window as skipped.
/// Compatible units such as `g` and `mg` share a base dimension. Incompatible
/// definitions remain in the schema as an explicit diagnostic.
pub fn schema_from_files(files: &[LoadedFile]) -> ChannelSchema {
    let mut schema = ChannelSchema::new();
    for file in files {
        for channel in file.tracks.iter().flat_map(|t| &t.channels) {
            if let Some(existing) = schema.get(&channel.name).cloned() {
                let mut merged = existing;
                if !channel_units_compatible(merged.unit.as_ref(), channel.unit.as_ref()) {
                    merged.conflicts.push(ChannelConflict::Unit {
                        expected: merged.unit.clone(),
                        found: channel.unit.clone(),
                    });
                }
                if merged.components != channel.components {
                    merged.conflicts.push(ChannelConflict::Components {
                        expected: merged.components.clone(),
                        found: channel.components.clone(),
                    });
                }
                let period_deg = channel.period.map(|period| period.get::<degree>());
                if merged.period_deg != period_deg {
                    merged.conflicts.push(ChannelConflict::Period {
                        expected_deg: merged.period_deg,
                        found_deg: period_deg,
                    });
                }
                schema.insert(&channel.name, merged);
                continue;
            }
            schema.insert(
                &channel.name,
                ChannelInfo {
                    unit: channel.unit.clone(),
                    period_deg: channel.period.map(|p| p.get::<degree>()),
                    components: channel.components.clone(),
                    conflicts: Vec::new(),
                },
            );
        }
    }
    schema
}

fn channel_units_compatible(
    existing: Option<&ChannelUnit>,
    incoming: Option<&ChannelUnit>,
) -> bool {
    match (existing, incoming) {
        (None, None) => true,
        (Some(existing), Some(incoming)) => {
            match (existing.as_recognized(), incoming.as_recognized()) {
                (Some(existing), Some(incoming)) => existing.quantity() == incoming.quantity(),
                (None, None) => {
                    existing.kind() == incoming.kind() && existing.label() == incoming.label()
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use uom::si::f64::Angle;

    use super::*;
    use crate::check::check_text;
    use crate::test_fixtures::{file_with_channels, scalar_channel, vector_channel};

    #[test]
    fn schema_from_files_types_a_channel_for_the_editor() {
        // A loaded g-unit accel channel resolves to an acceleration in the
        // editor: it compares to an acceleration literal and rejects a speed.
        let files = [file_with_channels(vec![scalar_channel(
            "accel",
            Some("g"),
            &[(0, 1.0)],
        )])];
        let schema = schema_from_files(&files);

        check_text("points | window 2 | where max(@accel) > 1 g", &schema)
            .expect("a g channel checks against an acceleration literal");
        let err = check_text("points | window 2 | where max(@accel) > 30 km/h", &schema)
            .expect_err("an acceleration cannot compare to a speed");
        assert!(err.message.contains("acceleration"), "{}", err.message);
    }

    #[test]
    fn schema_accepts_compatible_scales_and_rejects_incompatible_units() {
        let mg = vector_channel(
            "accel",
            Some("mg"),
            &["x", "y", "z"],
            &[(0, [20.0, 0.0, 0.0])],
        );
        let g = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [0.02, 0.0, 0.0])],
        );
        let compatible = [
            file_with_channels(vec![mg.clone()]),
            file_with_channels(vec![g]),
        ];
        let schema = schema_from_files(&compatible);
        check_text("points | window 2 | where max(@accel.x) > 10 mg", &schema)
            .expect("g and mg are compatible acceleration units");

        let degrees = vector_channel(
            "accel",
            Some("deg"),
            &["x", "y", "z"],
            &[(0, [20.0, 0.0, 0.0])],
        );
        let incompatible = [
            file_with_channels(vec![mg]),
            file_with_channels(vec![degrees]),
        ];
        let schema = schema_from_files(&incompatible);
        let err = check_text("points | window 2 | where max(@accel.x) > 10 mg", &schema)
            .expect_err("acceleration and angle units conflict");
        assert_eq!(
            err.message,
            "@accel has incompatible metadata across loaded files"
        );
        assert_eq!(err.help.as_deref(), Some("units mg and deg"));
    }

    #[test]
    fn schema_rejects_shape_component_order_and_period_conflicts() {
        let scalar = scalar_channel("sensor", Some("deg"), &[(0, 1.0)]);
        let vector = vector_channel(
            "sensor",
            Some("deg"),
            &["x", "y", "z"],
            &[(0, [1.0, 2.0, 3.0])],
        );
        let schema = schema_from_files(&[
            file_with_channels(vec![scalar]),
            file_with_channels(vec![vector]),
        ]);
        let err = check_text("points | window 2 | where max(@sensor) > 1 deg", &schema)
            .expect_err("scalar and vector definitions conflict");
        assert_eq!(
            err.help.as_deref(),
            Some("components [] and [\"x\", \"y\", \"z\"]")
        );

        let xyz = vector_channel(
            "sensor",
            Some("deg"),
            &["x", "y", "z"],
            &[(0, [1.0, 2.0, 3.0])],
        );
        let zyx = vector_channel(
            "sensor",
            Some("deg"),
            &["z", "y", "x"],
            &[(0, [1.0, 2.0, 3.0])],
        );
        let schema =
            schema_from_files(&[file_with_channels(vec![xyz]), file_with_channels(vec![zyx])]);
        let err = check_text("points | window 2 | where max(@sensor.x) > 1 deg", &schema)
            .expect_err("component order must agree");
        assert_eq!(
            err.help.as_deref(),
            Some("components [\"x\", \"y\", \"z\"] and [\"z\", \"y\", \"x\"]")
        );

        let mut linear = scalar_channel("sensor", Some("deg"), &[(0, 1.0)]);
        let mut circular = linear.clone();
        linear.period = None;
        circular.period = Some(Angle::new::<degree>(360.0));
        let schema = schema_from_files(&[
            file_with_channels(vec![linear]),
            file_with_channels(vec![circular]),
        ]);
        let err = check_text("points | window 2 | where max(@sensor) > 1 deg", &schema)
            .expect_err("linear and circular definitions conflict");
        assert_eq!(err.help.as_deref(), Some("periods None and Some(360.0)"));
    }
}
