# Review response

## Version policy

SDK versions are intentionally not bumped in feature pull requests.

The release workflow owns coordinated SDK version updates, so this branch preserves the existing versions while maintaining compatibility with the published C ABI.

## Resolved findings

### C ABI compatibility

The existing `GtdChannel`, `GtdChannelInfo`, `gtd_builder_add_channel`, and `gtd_nav_file_get_channel_info` layouts and behavior remain unchanged.

Custom-unit writes use the additive `gtd_builder_add_channel_with_unit_mode` function, and lossless reads use the additive length-query `gtd_nav_file_get_channel_unit` function.

The C tests include frozen 0.4 structure declarations, size and offset checks, and a call from the frozen input layout into the current library.

### Unit transport and legacy metadata

The shared unit model no longer has an FFI-derived label length limit.

The C accessor supports caller-sized buffers, and C, C++, and Python tests cover a 159-byte custom unit label.

Recognized aliases are parsed before the escape hatch is considered, valid unknown labels become display-only custom units, and malformed or normalization-changing legacy labels are preserved exactly as read.

Preserved malformed legacy labels cannot be supplied to the Rust channel writer as new metadata.

### Query schema and conflicts

The query schema now carries `ChannelUnit` directly instead of converting units to strings and reparsing them.

Schema merging records typed conflicts for incompatible unit dimensions, scalar versus vector shape, ordered component labels, and period semantics.

Queries that reference a conflicted channel are rejected with a diagnostic containing every recorded conflict.

Tests cover compatible SI scales and the requested shape, component-order, unit, and period conflicts.

### Typed SDK APIs

C++ now exposes `RecognizedUnit` and a validated `ChannelUnit` value instead of the invalid-state string and mode pair.

Python now exposes typed `Unit` constants, accepts `Unit`, `ChannelUnit`, and compatible recognized strings, and implements value equality and hashing for `ChannelUnit`.

The C, C++, and Python channel examples now demonstrate the typed or explicit custom-unit paths.

### Parser coverage and documentation

Property tests exercise arbitrary Unicode file labels, lossless preservation, normalization idempotence, and parser panic safety.

The recognized-unit catalog has exhaustive round-trip coverage.

`CHANGELOG_SDK.md` documents each language's migration, and the shared unit crate includes recognized, custom, conversion, and native-scale examples with a passing doctest.

## Findings outside this branch

The system-HDF5 rename implementation and history test comments cited by the review are present on the current trunk and are not changes introduced by `trunk...HEAD` after the rebase.

They were not modified as part of this channel-unit change and should be tracked against the history work on trunk separately.

## Verification

The shared-unit unit tests and doctest pass.

The focused query conflict test passes.

All C++ SDK test binaries, including the no-exceptions configuration, pass.

The Python SDK suite passes with 71 tests, strict mypy, Ruff, and all examples.
