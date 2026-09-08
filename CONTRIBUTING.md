# Contributing

The conventions live in the documents below.
Read the one that covers what you are changing.

## Documents

- [`CODE_STYLE.md`](CODE_STYLE.md) - Rust style: error handling and log levels, imports, comments, item order, naming, and the project vocabulary table.
- [`DESIGN.md`](DESIGN.md) - UI and text conventions: sentence casing, units, dates and times, dashes, button labels, disabled controls.
- [`AI_POLICY.md`](AI_POLICY.md) - using an AI tool on a contribution.
- [`RELEASES.md`](RELEASES.md) - the two release tracks, and what a version bump touches.
- [`CHANGELOG.md`](CHANGELOG.md) and [`CHANGELOG_SDK.md`](CHANGELOG_SDK.md) - a user-visible app change goes under `## Unreleased` in the first, an SDK change under `## [unreleased]` in the second.

## Bug reports and feature requests

File a GitHub issue with enough in it to reproduce the problem:

- **A `.gtd` file GeoTrace rejects, or reads wrongly.** Attach the file, or state what wrote it: which SDK (Rust, C, C++ or Python) and which version.
- **A `.gtd` file GeoTrace wrote that another tool rejects.** Attach the code that wrote it and the error the other tool reported.
- **A panic, or a value that the application shows wrongly.** Attach a test or an example program that reaches it.

A feature request states the problem to solve, an idea of how GeoTrace could support solving it, the alternatives, and the disadvantages.

## AI policy

Every use of an AI tool in a contribution follows the [AI policy](AI_POLICY.md).

## Pull requests

- A change over about 500 lines starts with an issue that agrees on the approach, which also makes the work in progress visible.
- A pull request covers one concern. Split a larger change into several.
- A refactoring and a mechanical change (a rename, moving code, formatting) are each a commit of their own, separate from the functional change they prepare.
- A change that a user sees is recorded in the changelog of its track, under the heading that the Documents section lists.
- Review your own pull request before requesting review.
- The code follows [`CODE_STYLE.md`](CODE_STYLE.md) and [`DESIGN.md`](DESIGN.md).

## Recipes

`just --list` prints every recipe. The ones a change goes through:

- `just check`, `just build`, `just run` - the root workspace.
- `just test` - the workspace test suite, through cargo-nextest.
- `just ci-essentials` - the fast CI subset, run before a commit.
- `just ci` - the full CI suite.

## Commits

Commits are atomic: each one does a single thing, builds and passes the tests on its own, and its message describes the change that it contains.
The pull request description covers the whole.

- The subject states the change in the imperative present tense, opens with a capital letter, ends without a period, and fits 72 characters.
- A blank line separates the subject from the body.
- A body states what changed and why, and only where the diff leaves the why unclear. Its lines fit 100 columns.
- The trailers a message may end with are `Signed-off-by`, `Co-authored-by`, `Reviewed-by`, `Tested-by`, `Acked-by` and `Assisted-by`, in that casing. A `Co-authored-by` line credits a person as `Name <address>`, and a tool goes in `Assisted-by` (see the [AI policy](AI_POLICY.md)).

Two checks read the message. `just qa::vale-added` covers the subject, the blank line, the body width and the trailers, and `just qa::check-commit-length` caps the body at 40 lines.

## Review

The bar for merging is high: every line added is a line to maintain from then on.
A review looks for:

- a pull request title and description that state what the change is for
- one thing per commit, and a message that says what it is
- code that reads plainly and follows [`CODE_STYLE.md`](CODE_STYLE.md)
- doc comments on every item that the SDK crates expose publicly
- a breaking change of the SDK recorded in the pull request description and in `CHANGELOG_SDK.md`
- the tests that the change calls for

## Licensing

GeoTrace is split-licensed: the application and the internal crates under AGPL-3.0-only, and the SDKs (`geotrace-sdk`, and the C, C++ and Python SDKs) under MIT.
A contribution is licensed under the license of the part that it touches, and the [README](README.md#license) states which part is which.
