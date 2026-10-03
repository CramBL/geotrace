use gt_logfile::LogLevelKind;

use super::{FilterPattern, FilterScope};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveFilterDraft {
    Hostname(String),
    Level(Option<LogLevelKind>),
    Message { text: String, regex: bool },
    Service(String),
}

impl LiveFilterDraft {
    pub fn empty(scope: FilterScope) -> Self {
        match scope {
            FilterScope::Hostname => Self::Hostname(String::new()),
            FilterScope::Level => Self::Level(None),
            FilterScope::Message => Self::default(),
            FilterScope::Service => Self::Service(String::new()),
        }
    }

    pub fn scope(&self) -> FilterScope {
        match self {
            Self::Hostname(_) => FilterScope::Hostname,
            Self::Level(_) => FilterScope::Level,
            Self::Message { .. } => FilterScope::Message,
            Self::Service(_) => FilterScope::Service,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Hostname(text) | Self::Service(text) | Self::Message { text, .. } => text,
            Self::Level(Some(level)) => level.as_ref(),
            Self::Level(None) => "",
        }
    }

    pub fn is_regex(&self) -> bool {
        matches!(self, Self::Message { regex: true, .. })
    }

    pub fn cleared(&self) -> Self {
        match self {
            Self::Message { regex, .. } => Self::Message {
                text: String::new(),
                regex: *regex,
            },
            _ => Self::empty(self.scope()),
        }
    }

    pub(crate) fn pattern(&self) -> Option<FilterPattern> {
        Some(match self {
            Self::Hostname(text) => FilterPattern::Hostname(text.clone()),
            Self::Level(None) => return None,
            Self::Level(Some(level)) => FilterPattern::Level(*level),
            Self::Message { text, regex } => FilterPattern::Message {
                text: text.clone(),
                regex: *regex,
            },
            Self::Service(text) => FilterPattern::Service(text.clone()),
        })
    }
}

impl Default for LiveFilterDraft {
    fn default() -> Self {
        Self::Message {
            text: String::new(),
            regex: false,
        }
    }
}
