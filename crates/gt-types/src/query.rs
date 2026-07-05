//! Shared query types used across crates.

/// How a query's matches change what the map shows.
///
/// The query language's `draw`/`keep`/`hide` stages select this; it is the
/// one place the parsed language (`gt-query`) and the renderer (`gt-map`,
/// via `gt-ui-types::QueryMatches`) agree on the vocabulary.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumCount,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
pub enum DisplayMode {
    /// Draw matches as halos over the track. The default when no display
    /// stage is written.
    #[default]
    Draw,
    /// Show only matching points; non-matching points are hidden.
    Keep,
    /// Hide matching points; the rest of the track stays.
    Hide,
}

impl DisplayMode {
    /// Whether a point is shown, given whether it matched the query:
    /// everything in `Draw`, only matches in `Keep`, everything but matches
    /// in `Hide`. The shared definition of what `keep`/`hide` mean, so every
    /// consumer (renderer, future exporters) agrees.
    pub fn shows(self, matched: bool) -> bool {
        match self {
            DisplayMode::Draw => true,
            DisplayMode::Keep => matched,
            DisplayMode::Hide => !matched,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mode_wire_names() {
        assert_eq!(DisplayMode::Draw.to_string(), "draw");
        assert_eq!(DisplayMode::Keep.to_string(), "keep");
        assert_eq!(DisplayMode::Hide.to_string(), "hide");
        assert_eq!("hide".parse(), Ok(DisplayMode::Hide));
    }

    #[test]
    fn shows_maps_each_mode() {
        // draw shows everything; keep shows matches; hide shows the rest.
        assert!(DisplayMode::Draw.shows(true) && DisplayMode::Draw.shows(false));
        assert!(DisplayMode::Keep.shows(true) && !DisplayMode::Keep.shows(false));
        assert!(!DisplayMode::Hide.shows(true) && DisplayMode::Hide.shows(false));
    }
}
