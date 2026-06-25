# Changelog

## [unreleased]

### Added

- Satellite utilization rate plotting: the share of in-view satellites (above an elevation mask) that the receiver uses in the fix, plotted combined and per constellation. Adjustable elevation mask in Settings, anomaly markers flagging satellites used below the mask, and an "Advanced" toggle that reveals these metrics (hidden by default).

### Fixed

- The time-range filter now also hides filtered-out points from map hover and click, the plot cross-highlight, and double-click zoom-to-fit. Previously they were only hidden visually but stayed interactive.

## [0.1.0] - 2026-06-21

Initial release
