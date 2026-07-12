# Code style

## Rust code

### Avoid `unsafe`

`unsafe` code should be only used when necessary, and should be carefully scrutinized during PR reviews.

### Avoid `unwrap`, `expect` etc.
The code should never panic or crash, which means that any instance of `unwrap` or `expect` is a potential time-bomb. Even if you structured your code to make them impossible, any reader will have to read the code very carefully to prove to themselves that an `unwrap` won't panic. Often you can instead rewrite your code so as to avoid it. The same goes for indexing into a slice (which will panic on out-of-bounds) - it is often preferable to use `.get()`.

For instance:

``` rust
let first = if vec.is_empty() {
    return;
} else {
    vec[0]
};
```
can be better written as:

``` rust
let Some(first) = vec.get(0) else {
    return;
};
```

### Iterators
Be careful when iterating over `HashSet`s and `HashMap`s, as the order is non-deterministic.
Whenever you return a list or an iterator, sort it first.
If you don't want to sort it for performance reasons, you MUST put `unsorted` in the  name as a warning.

### Error handling and logging

* An error should never happen in silence.
* Validate code invariants using `assert!` or `debug_assert!`.
* Validate user data and return errors using [`thiserror`](https://crates.io/crates/thiserror).
* Attach context to errors as they bubble up the stack using [`anyhow`](https://crates.io/crates/anyhow).
* If a problem is recoverable, use `log::warn!`.
* If an event is of interest to the user, log it using `log::info!`.
* The code should only panic if there is a bug in the code.
* Never ignore an error: either pass it on, or log it.
* Handle each error exactly once. If you log it, don't pass it on. If you pass it on, don't log it.

Strive to encode code invariants and contracts in the type system as much as possible. So if a vector cannot be empty, consider using [`vec1`](https://crates.io/crates/vec1). [Parse, don’t validate](https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/).

Some contracts cannot be enforced using the type system. In those cases you should explicitly enforce them using `assert` (self-documenting code) and in documentation (if it is part of a public API).

### Log levels

The log is for several distinct users:
* The application user
* The application programmer
* The library user
* The library programmer

We are all sharing the same log stream, so we must cooperate carefully.

#### `ERROR`
This is for _unrecoverable_ problems. The application or library couldn't complete an operation.

Libraries should ideally not log `ERROR`, but instead return `Err` in a `Result`, but there are rare cases where returning a `Result` isn't possible (e.g. then doing an operation in a background task).

Application can "handle" `Err`ors by logging them as `ERROR` (perhaps in addition to showing a popup, if this is a GUI app).

Use this log level whenever some data is lost, even if you continue processing other data.

Examples: failing to write a file, failing to read parts of a file.

#### `WARNING`
This is for _recoverable_ problems. The operation completed, but couldn't do exactly what it was instructed to do.

Sometimes an `Err` is handled by logging it as `WARNING` and then running some fallback code.

Warnings are also used for thing that _may_ be an error, but it could be intended.

If data is lost, it is an error and NOT a warning.

#### `INFO`
This is the default verbosity level. This should mostly be used _only by application code_ to write interesting and rare things to the application user. For instance, you may perhaps log that a file was saved to specific path, or where the default configuration was read from. These things lets application users understand what the application is doing, and debug their use of the application.

#### `DEBUG`
This is a level you opt-in to to debug either an application or a library. These are logged when high-level operations are performed (e.g. texture creation). If it is likely going to be logged each frame, move it to `TRACE` instead.

#### `TRACE`
This is the last-resort log level, and mostly for debugging libraries or the use of libraries. Here any and all spam goes, logging low-level operations.

The distinction between `DEBUG` and `TRACE` is the least clear. Here we use a rule of thumb: if it generates a lot of continuous logging (e.g. each frame), it should go to `TRACE`.


### Warning reporter pattern
For reporting warnings (or partial-failures) up the call-stack, we like the _reporter pattern_:

```rs
struct WarningReporter {
    reports: Mutex<Vec<Warning>>,
}

pub fn thing_that_can_produce_warnings(reporter: &WarningReporter, other_paramets: …) -> Result<…> {}
```

The important parts of this pattern is:
* Accumulate warnings and then continue
* Interior mutability, so we can share the reporter with child threads
* Structured warnings (more than just a String!)

We use this for _partial failures_, when something went wrong but we don't want to abort, but instead continue with best-effort.

We prefer this pattern complex return types (`(Vec<Warning>, Object)`), because the reporter pattern is often a lot less syntactically noisy in Rust.
It is also easy to ignore part of a return-type, but it is harder to ignore an extra parameter. Thus we force ourselves to handle warnings.

This allows code like this:

```rs
fn some_panel_ui(ctx: &ViewerContext, ui: &mut Ui) {
    let reporter = WarningReporter::default();
    let object = do_some_query(&reporter, …)?;
    object.ui(ui);
    if reporter.is_missing_chunks() {
        ui.loading_indicator("Doing query");
    }
    if !reporter.warnings().is_empty() {
        warnings_ui(reporter.warnings());
    }
}
````

### Libraries
We use [`thiserror`](https://crates.io/crates/thiserror) for errors in our libraries, and [`anyhow`](https://crates.io/crates/anyhow) for type-erased errors in applications.

### Style
We follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/about.html).

We use `rust fmt` with default settings.

We have blank lines before functions, types, `impl` blocks, and docstrings.

We format comments `// Like this`, and `//not like this`.

Never use ASCII art "section divider" comments such as `// ─── Foo ───`, `# ─── Foo ───`, or any variant made of box-drawing characters or repeated dashes.
They add visual noise, break cleanly in diffs, and communicate nothing that a blank line or a normal comment does not.
The only exception is when the comment is literally drawing a protocol diagram, a data layout, or another spatial structure for illustrative purposes.

When importing a `trait` to use its trait methods, do this: `use Trait as _;`. That lets the reader know why you imported it, even though it seems unused.

Always import **types** fully so they can be used by their short name at the call site.
This applies even when the type is from an external crate - add the `use` and drop the crate qualifier at the call site:

```rust
// Good
use std::collections::HashMap;
let m: HashMap<_, _> = …;

// Good - same rule for external crates
use some_crate::SomeType;
fn foo(x: SomeType) { … }

// Bad - qualifies a type rather than importing it
let m: std::collections::HashMap<_, _> = …;
fn foo(x: some_crate::SomeType) { … }
```

**Exception for GUI Types:**
Unambiguous UI components should follow the standard type rule and be fully imported (e.g., `use egui::{Button, RichText};`). However, `egui` contains many generic types (`Context`, `Response`, `Id`, `Rect`). To avoid namespace pollution and confusion, these generic types, as well as the ubiquitous `egui::Ui`, should be qualified at the module level.

Never fully import **functions** - always retain at least the parent module so the call site is self-documenting.
Import the parent module and qualify the call with it, even across crates:

```rust
// Good
use std::fs;
fs::read_to_string(path)?;

// Good - same rule for external crates: import the module, qualify the call
use some_crate::utils;
utils::helper(…);

// Bad - hides where the function comes from
use std::fs::read_to_string;
read_to_string(path)?;

// Bad - qualifies at the crate level instead of the parent module
some_crate::utils::helper(…);
```

For very common functions (e.g. `mem::swap`, `iter::once`) retaining one module level is sufficient; for less familiar ones, keep more context.

When intentionally ignoring a `Result`, prefer `foo().ok();` over `let _ = foo();`. The former shows what is happening, and will fail to compile if `foo`:s return type ever changes.

Never use `foo().unwrap_or(())` to discard a `Result` - it is more verbose than `.ok()` and hides the intent behind an API designed for providing fallback values, not silently dropping errors.

We group and order imports (`use` statements) by `std`, other crates, and lastly own `crate` and `super`. This corresponds to [`StdExternalCrate`](https://rust-lang.github.io/rustfmt/?version=v1.8.0&search=group#StdExternalCrate%5C%3A).

We group our `use` statements by module, e.g. `crate_name::module::{a, b, c}`. This is a compromise, being rather terse while still avoiding excessive merge conflicts. See [the cargofmt docs](https://rust-lang.github.io/rustfmt/?version=v1.8.0&search=group#Module%5C%3A) for details.

Use the destructor syntax (`let Self { a, b, c} = self;`) whenever you're accessing most of (or all) of the fields of a struct.

### Misc
Use debug-formatting (`{:?}`) when logging strings in logs and error messages. This will surround the string with quotes and escape newlines, tabs, etc. For instance: `log::warn!("Unknown key: {key:?}");`.

Use `{:#}` when displaying an error - NOT `Debug`/`{:?}`.

We make extensive use of snapshot testing. To work around non-deterministic values, such as TUIDs (time-prefixed unique IDs), many types (should) offer `std::fmt::Display` implementations with redactions that can be access via an overloaded `-` formatting option:

```rs
println!("{:-}, value"); // The `-` option stands for redaction.
```

## Naming
We prefer `snake_case` to `kebab-case` for most things (e.g. crate names, crate features, …). `snake_case` is a valid identifier in almost any programming language, while `kebab-case` is not. This means one can use the same `snake_case` identifier everywhere, and not think about whether it needs to be written as `snake_case` in some circumstances.

When in doubt, be explicit. BAD: `id`. GOOD: `msg_id`.

Be terse when it doesn't hurt readability. BAD: `message_identifier`. GOOD: `msg_id`.

Avoid negations in names. A lot of people struggle with double negations, so things like `non_blocking = false` and `if !non_blocking { … }` can become a source of confusion and will slow down most readers. So prefer `connected` over `disconnected`, `initialized` over `uninitialized` etc.

For UI functions (functions taking an `&mut egui::Ui` argument), we use the name `ui` or `_ui` suffix, e.g. `blueprint_ui(…)` or `blueprint.ui(…)`.

### Project vocabulary

These are the canonical terms used in code, docs, and UI.
Using consistent names keeps grep, autocomplete, and mental models aligned.

| Term | Meaning | Examples in code |
|------|---------|-----------------|
| **file** | A single loaded `.gtd` file; maps to one `LoadedFile`. | `FileIdx`, `FileNode`, `fi` |
| **track** | A contiguous GPS recording within a file (what the user calls a "track"). | `LoadedTrack`, `TrackIdx`, `TrackRef`, `ti` |
| **point** | A single data point within a track. | `PointIdx`, `SpatialPoint`, `pi` |
| **track ref** (`TrackRef`) | Typed (file-index, track-index) pair that uniquely addresses a track. Fields: `fi: FileIdx`, `index: TrackIdx`. | `TrackRef::new(fi, ti)` |
| **data point ref** (`DataPointRef`) | Typed address of a single rendered point. Fields: `track: TrackRef`, `category`, `point_index`. | sticky/hover highlight |
| **event marker** | A timestamped event associated with a track (e.g. `power/boot`). | `EventMarker`, `event_markers` |
| **custom marker** | A user-placed geographic annotation. | `CustomMarker`, `custom_markers` |
| **generated marker** | A marker derived automatically from data (e.g. trip start/end). | `GeneratedMarker`, `generated_markers` |
| **visibility** | Whether a file/track/marker is shown on the map. Controlled via `Visibility` / `GlobalFilter`. | `track_visible`, `show_only_track` |
| **highlight** | Transient hover or sticky selection state. | `MapHighlight`, `HighlightScope` |
| **polyline span** | A maximal stretch of consecutive on-screen track points painted as one unbroken polyline; viewport culling splits a track into spans, and key changes (e.g. fix-quality color) split spans into sub-spans. Renderer-internal to `gt-map`. | `PolylineSpans`, `VisiblePath::Spans`, `split_key_spans` |
| **query** | A short declarative pipeline the user writes and runs over loaded data. Defined by the `gt-query` crate (lex → parse → check → run) and driven from the query window. | `gt-query`, `QueryWindow`, `CheckedQuery` |
| **stage** | One step of a query pipeline, separated by `\|`: the source (`points`), `with`, `window`, `where`, `draw`, `table`. | parser stage methods in `gt-query` |
| **match** | A maximal stretch of consecutive points satisfying a query, drawn on the map as a halo. Not "trace" (TPV trace), "span" (renderer), or "segment" (track building). | `QueryMatches`, `TrackMatches`, `match_at` |
| **query history** | The remembered list of previously run queries. Always written in full - bare "history" means the recording-history database (`gt-history`). | `QueryHistoryEntry`, `QuerySettings::history` |
| **snap** / **snap to road** | Matching a track against the OpenStreetMap road network (Valhalla map matching), and the act of requesting it for one track. Not "match" (query matches) or "trace" (TPV traces). | `gt-snap`, `SnapScheduler`, `SnapResult` |
| **snapped track** | The matched road geometry drawn on the map alongside the recorded track. Not "ghost track" - a ghost is a heading-less nav point. | `SnappedTracks`, `SnappedTrackSegment` |
| **snap error** | Distance in meters from a recorded point to its snapped position. | `MetricKind::SnapError`, `SnapErrorSeries` |
| **snapped / interpolated / unsnapped** | Per-point match kind, mirroring Valhalla's `matched` / `interpolated` / `unmatched` wire names. | `SnapPointKind`, `SnapErrorKind`, `SnapKindCounts` |
| **discontinuity** | A stretch Valhalla could not connect through the road network; rendered as a gap in the snapped track. | `begin_route_discontinuity`, `snapped_track::point_groups` |
| **travel mode** | Optional `.gtd` metadata declaring the recording platform (car, bicycle, boat, ...). Declared by the recorder via the SDKs; the app derives the default snap costing from it. | `TravelMode`, `meta_travel_mode`, `resolve_costing` |

Terms to avoid and their replacements:

| Avoid | Use instead |
|-------|------------|
| trip | track |
| TripRef | TrackRef |
| TripNode | TrackNode |

### Be over-explicit in stringly typed situations

Avoid vague names like "address". Prefer one of:

* `ip`
* `ip_port`
* `url`
* `email`
* …

### Avoid magic strings and numbers

Use named constants.

Never write raw Unicode escape sequences (`\u{XXXX}`) for special characters directly inside string literals or `format!` macros.
Such escapes are opaque to reviewers and make the intent hard to see at a glance.
Instead, use:

- An [`egui_phosphor`](https://docs.rs/egui-phosphor) icon constant when the character is a visual icon. Because these constants are highly useful in `format!` macros (e.g. `format!("{ICON_MAGNIFYING_GLASS} Search")`), you should fully import or alias them (e.g. `use egui_phosphor::regular::MAGNIFYING_GLASS as ICON_MAGNIFYING_GLASS;`) instead of writing out the full path at the call site.
- A named `const &str` when the character is a typographic symbol that appears inline with text - for example `const EM_DASH: &str = "—";` or `const ELLIPSIS: &str = "…";`.
  Define the constant in the narrowest scope that covers all callers (file-level `const` in the module that owns the UI, crate-level if shared across files in a crate).
  The constant body may contain the literal Unicode character directly (the restriction is on escape sequences, not on non-ASCII characters in source).
