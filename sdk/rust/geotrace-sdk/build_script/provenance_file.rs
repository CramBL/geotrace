const COMMIT_HASH_LEN: usize = 40;

/// The commit hash and the committer timestamp in UTC, or what makes
/// `contents` unusable: a commit hash on the first line, an RFC 3339 timestamp
/// on the second.
fn parse_provenance_file(contents: &str) -> Result<(&str, String), String> {
    let mut lines = contents.lines();
    let commit = lines.next().unwrap_or_default().trim();
    let commit_time = lines.next().unwrap_or_default().trim();

    if commit.len() != COMMIT_HASH_LEN || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("line 1 is not a commit hash: {commit:?}"));
    }
    let commit_time = chrono::DateTime::parse_from_rfc3339(commit_time)
        .map_err(|err| format!("line 2 is not an RFC 3339 timestamp: {err}"))?
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    Ok((commit, commit_time))
}
