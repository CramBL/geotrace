use std::fmt;

use strum::IntoEnumIterator as _;

use crate::ast::BinaryOp;
use crate::construct::{self, ConstructKind};

const GUIDE: &str = include_str!("reference.md");

pub fn query_language_reference() -> String {
    QueryLanguageReference.to_string()
}

struct QueryLanguageReference;

impl fmt::Display for QueryLanguageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(GUIDE)?;
        self::write_binary_operators(f)?;
        for (kind, heading) in [
            (ConstructKind::Source, "Point source"),
            (ConstructKind::Stage, "Stages"),
            (ConstructKind::Mode, "Display modes"),
            (ConstructKind::Param, "Analysis parameters"),
            (ConstructKind::Connective, "Logical operators"),
            (ConstructKind::Function, "Functions"),
            (ConstructKind::Unit, "Units"),
        ] {
            write!(f, "\n## {heading}\n\n")?;
            for construct in construct::catalog()
                .iter()
                .filter(|entry| entry.kind == kind)
            {
                writeln!(f, "- `{}` ({})", construct.name, construct.summary)?;
                if !construct.doc.is_empty() {
                    write!(f, "\n  {}\n\n", construct.doc)?;
                }
            }
        }
        Ok(())
    }
}

fn write_binary_operators(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let mut comparison = Vec::new();
    let mut arithmetic = Vec::new();
    for op in BinaryOp::iter() {
        let group = match op {
            BinaryOp::And | BinaryOp::Or => continue,
            BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::Eq
            | BinaryOp::Ne => &mut comparison,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => &mut arithmetic,
        };
        group.push(format!("`{}`", op.text()));
    }
    write!(
        f,
        "\n## Numeric operators\n\n- Comparisons ({})\n- Arithmetic ({})\n",
        comparison.join(", "),
        arithmetic.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::{ChannelInfo, Unit, check, parser, test_util};

    #[rstest]
    #[case::lf("\n")]
    #[case::crlf("\r\n")]
    fn every_complete_reference_example_checks(#[case] line_ending: &str) {
        let mut schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
        schema.insert(
            "speed",
            ChannelInfo {
                unit: Some(Unit::KM_PER_H.into()),
                period_deg: None,
                components: Vec::new(),
                conflicts: Vec::new(),
            },
        );
        let mut points = false;
        let mut channels = false;
        let reference = super::query_language_reference()
            .replace("\r\n", "\n")
            .replace('\n', line_ending);
        for block in reference.split("```").skip(1).step_by(2) {
            let (language, query) = block.split_once('\n').expect("query code block");
            assert_eq!(language.trim_end_matches('\r'), "text");
            let parsed = parser::parse(query).unwrap_or_else(|error| panic!("{query}: {error:?}"));
            let checked =
                check::check(&parsed, &schema).unwrap_or_else(|error| panic!("{query}: {error:?}"));
            if checked.is_channel_source() {
                channels = true;
            } else {
                points = true;
            }
        }
        assert!(points && channels);
    }
}
