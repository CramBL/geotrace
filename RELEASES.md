# Releasing

GeoTrace has two independent release tracks: the **GUI app** and the **SDKs** (Rust, Python, C/C++).
They version and ship separately, so bump and tag only the track you are releasing.

A pushed tag drives the release: the GitHub release and all publishing are automated off it.
Two rules hold for both tracks:

- **The manifest version must exactly equal the tag's version.**
  cargo-dist (GUI) and the SDK version guard both refuse to release a tag whose version does not match the version in the manifests.
  So a prerelease is not just a tag suffix.
  The manifest must carry the `-rc.N` suffix too, which means the prerelease and the final release are _different commits_.
- **Bump on a branch, merge via PR, then tag the merged commit** - never commit a version bump straight to `trunk`.

## Changelog

Each track has a changelog: `CHANGELOG.md` (GUI) and `CHANGELOG_SDK.md` (SDK).
Record app changes under `## Unreleased` in `CHANGELOG.md` and SDK changes under `## [unreleased]` in `CHANGELOG_SDK.md`.
That section becomes the GitHub release body.

The release flow promotes it for you.
`just qa::bump-app` turns `## Unreleased` into `## X.Y.Z - YYYY-MM-DD`, which the app release metadata workflow prepends to the GitHub release body after cargo-dist creates the release.
`just qa::bump-sdk` turns `## [unreleased]` into `## [X.Y.Z] - YYYY-MM-DD` and leaves a fresh empty `## [unreleased]` on top.
A prerelease (`X.Y.Z-rc.N`) and its final release share one core-version section.
The `--expect` guards refuse to tag if that section is missing.

## Scripted flow

The `just release::*` recipes walk these steps interactively from an up-to-date, clean `trunk`, printing each command as `<command> ?` and running it when you press Enter (`s` skips, `q` quits):

- `just release::app X.Y.Z` / `just release::sdk X.Y.Z` - regular release (bump, PR, then tag the merged commit).
  It pauses for you to merge the PR.
- `just release::app-prerelease X.Y.Z-rc.N` / `just release::sdk-prerelease X.Y.Z-rc.N` - prerelease dry run (linear, no PR).

The sections below document what each step does, for running it by hand.

## GUI app

Bump on a branch and open a PR:

```sh
git switch -c release/app-vX.Y.Z
just qa::bump-app X.Y.Z      # edits the workspace version + promotes CHANGELOG.md
git commit -am "release app vX.Y.Z"
git push -u origin release/app-vX.Y.Z
```

Opening the PR runs CI and a cargo-dist plan dry-run (`pr-run-mode = "upload"` builds the installers too, without releasing).
Merge when green, then tag the merged commit:

```sh
git switch trunk && git pull
git tag app/vX.Y.Z
git push origin app/vX.Y.Z
```

`.github/workflows/app-release.yml` (cargo dist) builds the shell/PowerShell installers, the MSI, and the Homebrew formula, then publishes the GitHub release.
Apps installed via the shell/PowerShell installer offer the update on next launch.

If the manifest already carries the version you are releasing (e.g. the very first release, where `trunk` is already at it), there is nothing to bump - skip the PR and tag `trunk` directly.

## SDKs

The Rust, Python, and C/C++ SDKs version in lockstep.
Same shape - bump on a branch, PR, merge, then tag:

```sh
git switch -c release/sdk-vX.Y.Z
just qa::bump-sdk X.Y.Z      # edits every SDK version spot + promotes CHANGELOG_SDK.md (fails if it can't)
git commit -am "release sdk vX.Y.Z"
git push -u origin release/sdk-vX.Y.Z
# open the PR, merge, then:
git switch trunk && git pull
git tag geotrace-sdk-vX.Y.Z
git push origin geotrace-sdk-vX.Y.Z
```

`bump-sdk` rewrites the version macros in `sdk/rust/geotrace-c/cbindgen.toml` and in the header it generates, `sdk/c/geotrace.h`, leaving the two equal to what `just sdk-c-header` writes.

`.github/workflows/release-sdk.yml` builds the C/C++ archives, publishes to crates.io and PyPI, updates the `geotrace-c` Homebrew formula, and publishes the GitHub release.
A version guard fails the run before publishing if the manifests disagree or do not match the tag.
Publishing is idempotent, so a partially failed run can be re-run safely.

## Prereleases

A prerelease is a full dry run of the real release: the GitHub release is flagged as a prerelease and the smoke tests run, but nothing is published to a public channel.
Because the manifest version must match the tag, cut it from the release branch _before_ promoting to the final version:

```sh
# on the release branch
just qa::bump-app X.Y.Z-rc.1     # or: just qa::bump-sdk X.Y.Z-rc.1
git commit -am "release app vX.Y.Z-rc.1"
git push -u origin release/app-vX.Y.Z
git tag app/vX.Y.Z-rc.1          # or geotrace-sdk-vX.Y.Z-rc.1
git push origin app/vX.Y.Z-rc.1
# when the prerelease + smoke are green, promote to the final version:
just qa::bump-app X.Y.Z
git commit -am "release app vX.Y.Z"
# open the PR, merge, then tag the final version (see above)
```

The prerelease tag points at the `-rc.N` commit on the branch.
Only the final, clean-version commit is merged to `trunk` and tagged there.
To keep the public channels clean, a prerelease publishes to none of them: the GUI skips the Homebrew formula, and the SDK skips crates.io, PyPI, and Homebrew.
The smoke tests still verify every artifact by consuming it directly: the GUI installers and the C/C++ archives from the prerelease itself, the Rust crate from the tagged git source, and the Python wheel attached to the prerelease (installed with `pip --find-links`).
The GUI updater ignores prereleases.

## After a release

`.github/workflows/release-smoke.yml` installs every artifact on every platform (running `geotrace --version` and building SDK consumers) to catch a broken release fast.
It runs automatically on every release, including prereleases, but the two tracks trigger it differently because of a GitHub rule: a release created with the built-in `GITHUB_TOKEN` does not emit a `release: published` event that can start another workflow.

- The **SDK** release creates its GitHub release with a repo-scoped PAT (`GEOTRACE_WITH_REPO_SCOPE`), so its `release: published` event triggers the smoke workflow directly.
- The **GUI** release is created by cargo-dist with `GITHUB_TOKEN`, so the smoke workflow instead runs when the cargo-dist `Release` workflow completes, via a `workflow_run` trigger filtered to `app/**` tags. (A dist post-announce job cannot be used: it would be skipped on prereleases, because it inherits the skip from the Homebrew publish job that dist gates to stable releases.)

Re-run it for any tag from the Actions tab - `Run workflow` dispatches `release-smoke.yml` with a `tag` input.
