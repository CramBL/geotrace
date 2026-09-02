use chrono::{DateTime, Utc};

/// Name of the file attribute holding [`Meta::sdk_version`](crate::Meta::sdk_version).
pub const SDK_VERSION_ATTR: &str = "sdk_version";

/// Name of the file attribute holding [`Meta::sdk_git_commit`](crate::Meta::sdk_git_commit).
pub const SDK_GIT_COMMIT_ATTR: &str = "sdk_git_commit";

/// Name of the file attribute holding [`Meta::sdk_commit_time`](crate::Meta::sdk_commit_time).
pub const SDK_COMMIT_TIME_ATTR: &str = "sdk_commit_time";

/// The geotrace commit a released SDK build was made from, and its committer
/// timestamp in RFC 3339.
#[derive(Clone, Copy)]
pub(crate) struct BuildProvenance {
    pub(crate) commit: &'static str,
    pub(crate) commit_time: &'static str,
}

/// What `build.rs` read from `build_provenance.txt`, `None` in a build without
/// that file.
pub(crate) const PROVENANCE: Option<BuildProvenance> = match (
    option_env!("GEOTRACE_SDK_GIT_COMMIT"),
    option_env!("GEOTRACE_SDK_COMMIT_TIME"),
) {
    (Some(commit), Some(commit_time)) => Some(BuildProvenance {
        commit,
        commit_time,
    }),
    _ => None,
};

pub(crate) fn commit_time() -> Option<DateTime<Utc>> {
    parse_rfc3339("the commit time build.rs wrote", PROVENANCE?.commit_time)
}

/// `None` for a `raw` that is not RFC 3339, logged as a warning naming `source`.
pub(crate) fn parse_rfc3339(source: &str, raw: &str) -> Option<DateTime<Utc>> {
    match DateTime::parse_from_rfc3339(raw) {
        Ok(time) => Some(time.with_timezone(&Utc)),
        Err(err) => {
            log::warn!("{source} is {raw:?}, which is not RFC 3339: {err}");
            None
        }
    }
}
