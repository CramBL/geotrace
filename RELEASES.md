# Releasing

GeoTrace has two independent release tracks: the **GUI app** and the **SDKs** (Rust, Python, C/C++).
They version and ship separately, so bump and tag only the track you are releasing.

Tags drive everything; the GitHub release and all publishing are automated.

## GUI app

```sh
just qa::bump-app X.Y.Z      # edits the workspace version
git commit -am "release app vX.Y.Z"
git tag app/vX.Y.Z
git push origin app/vX.Y.Z
```

`.github/workflows/app-release.yml` (cargo dist) builds the shell/PowerShell installers, the MSI, and the Homebrew formula, then publishes the GitHub release.
Apps installed via the shell/PowerShell installer offer the update on next launch.

## SDKs

The Rust, Python, and C/C++ SDKs version in lockstep.

```sh
just qa::bump-sdk X.Y.Z      # edits every SDK version spot (fails if it can't)
git commit -am "release sdk vX.Y.Z"
git tag geotrace-sdk-vX.Y.Z
git push origin geotrace-sdk-vX.Y.Z
```

`.github/workflows/release-sdk.yml` builds the C/C++ archives, publishes to crates.io and PyPI, updates the `geotrace-c` Homebrew formula, and publishes the GitHub release.
A version guard fails the run before publishing if the manifests disagree or do not match the tag.
Publishing is idempotent, so a partially failed run can be re-run safely.

## Prereleases

Add a suffix: `app/vX.Y.Z-rc.1` or `geotrace-sdk-vX.Y.Z-alpha.1`.
The GitHub release is flagged as a prerelease, and the smoke tests below run against it — so a prerelease tag is a full dry run of the release before you cut the real one.

To keep the public channels clean, a prerelease publishes to none of them: the GUI skips the Homebrew formula, and the SDK skips crates.io, PyPI, and Homebrew.
The smoke tests still verify every artifact by consuming it directly: the GUI installers and the C/C++ archives from the prerelease itself, the Rust crate from the tagged git source, and the Python wheel attached to the prerelease (installed with `pip --find-links`).
The GUI updater ignores prereleases.

## After a release

`.github/workflows/release-smoke.yml` runs automatically on every release, including prereleases, and installs every artifact on every platform — running `geotrace --version` and building SDK consumers — to catch a broken release fast.
Re-run it for any tag from the Actions tab (`Run workflow`) if needed.

## One-time setup

- Repository secrets: `CARGO_REGISTRY_TOKEN` and `HOMEBREW_TAP_TOKEN` (push access to `CramBL/homebrew-tap`).
- A GitHub environment named `pypi`, with this repository registered as a PyPI Trusted Publisher.
- Releases are currently unsigned; see the install notes in [`README.md`](README.md) for the Gatekeeper / SmartScreen workaround.
