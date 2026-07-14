# Changelog

## Unreleased

### Added

- Snap to road: match a track against the OpenStreetMap road network from its side-panel action.
  The match draws as a dashed line on the map (toggleable per track), and the new "Snap error (m)" plot metric shows each point's distance to the road, right next to EPH.
  Recordings that declare a travel mode are matched against the right network automatically.
  Nothing is uploaded without a one-time consent naming the server, which is configurable in Settings (default: the public instance hosted by FOSSGIS e.V.).
- Snap to road can now run automatically: with "Snap to road automatically" on, tracks are matched as you load and show them, visible tracks first.
  The choice is part of the consent dialog and can be changed in Settings anytime; nothing changes for existing setups until you opt in.
- Snap results of recordings stored in history are kept across sessions - reopening a recording shows its snapped track without re-uploading anything.
- Results that were matched under different settings than the current ones are marked stale (amber icon, the differences listed on hover) with a one-click re-run.
  Failed stretches and match warnings now show in the same hover.
- Hovering the snapped line shows the matched road's name, class, speed limit, and surface.
  Zoomed in far enough, thin whiskers connect each recorded point to its position on the road, making the error visible on the map itself.
- "Snap again as" in a track's context menu re-matches it as auto, bicycle, or pedestrian - also the escape hatch when a recording declares the wrong travel mode.
- Advanced snap settings: search radius, turn penalty, and a GPS accuracy override, for tuning the matcher against unusually noisy receivers.
- The map display toggle gains a "Snapped tracks" row, and snapped-track rendering no longer slows the map down at street-level zoom.
- A File menu in the top-left with Open and the new About dialog (version and data attributions).
- Settings now has a "Recording name" template that sets how recordings are labelled in the side panel.
  Combine `{title}`, `{device}`, `{identity}` and `{filename}`; empty fields and their separators drop out.
  Defaults to the filename, as before.
- Recordings with metadata now show a note icon in the side panel; click it to open a resizable dialog with the title, device, identity and notes.
- The History window marks recordings that carry metadata with a note icon and shows their title, device and notes on hover.
- Recordings declaring a travel mode (car, bicycle, boat, ...) show it in the recording details dialog and the History window metadata hover.
- Recording identities can be renamed from the History window: click the pencil, type a new name, press Enter. Recordings sharing the old name move together, and renaming onto an existing name merges into it.

### Changed

- Each recording's distance and duration in the side panel now carry a road and clock icon.

### Fixed

- A long recording name no longer hides its distance and duration in the side panel, only the name is truncated.
- The History window and the dialogs that list recording identities or paths (prune and auto-prune previews, the remove-items, data-quality, and load-progress windows) no longer stretch off-screen for a long name, the text truncates with the full value shown on hover.
- Satellite counts, signal quality, and other status colors are now readable in light mode, previously the yellow and green shades washed out against the light background.
- Plot series lines and query editor syntax highlighting are now readable in light mode too, the plot also gets a faint-grey canvas so the lines stand out.
- Focusing a track no longer washes the whole map to white, the dimming is now a gentle veil, lighter still in dark mode.

## 0.5.1 - 2026-07-09

### Changed

- When multiple recordings share a common directory prefix, that prefix is omitted from their labels in the side panel so only the distinct part of each name is shown.
  The full path is still visible when hovering the label.

### Fixed

- Side panel file labels no longer force the panel to grow when a recording has a long name, the label is now truncated at the available width.

## 0.5.0 - 2026-07-09

### Added

- Map display toggles: an eye button above the map/satellite switch opens a popup where each kind of map element (tracks, track points, satellite labels, the three marker types, query highlights) can be hidden or shown, with a live count of each. Hiding is purely visual, filters, the track list, and deletion are unaffected. `only` (or alt-click) shows a single category, pressing it again restores the previous state. When anything is hidden the eye turns into a tinted crossed-out eye as a reminder.
- Plot display settings: a gear button in the plot's filter row opens a small popup with an adjustable line width and the grid toggle (previously its own button in the row).

### Changed

- Plot lines are slightly thinner by default so plots with many metrics enabled stay readable.
- Satellite-count labels are now placed at diagnostically relevant points (fix-quality changes, signal dips, recoveries after loss) instead of at fixed intervals, and no longer shuffle while panning the map.

### Fixed

- History now stores producer-supplied recording identities as encoded HDF5 group names, so path-like `.gtd` identities no longer create hidden root-level database groups.
- Opening an existing history database repairs recordings that were previously stored outside `/by_identity`, and history insert failures now log the database path, identity, and visibility check result.

## 0.4.0 - 2026-07-08

### Added

- Query window: write a short pipeline (e.g. `points | window 10 | where avg(velocity) > 30 km/h`) to find stretches of the loaded data. Includes unit-aware expressions, syntax highlighting, a built-in examples list, and a persisted query history.
- Query display modes: besides drawing halos on matches, a query can end with `keep` (show only the matching points) or `hide` (remove the matching points).
- Query editor autocomplete: as you type, a popup under the caret lists the constructs valid at that position (fuzzy-matched), with Enter or Tab to accept, arrows to choose, and Esc to dismiss. Unit suggestions are restricted to the compared value's quantity.
- Query editor hover documentation: hovering a construct shows a Rust-doc-style tooltip with its summary, a fuller explanation, and example usage.
- Query results cross-highlight: hovering a match in the results table draws a halo band over its points on the map and shades its time span on the plot. Clicking a table row pins the point's map popup, double-clicking centers the map on it.
- Channel plots: recordings carrying ad-hoc sensor channels get a "Channels" toggle in the plot's filter row, revealing one chip per channel. A vector channel draws one line per component (`accel.x/y/z`), on the channel's own sample rate. Toggles persist like the metric chips.

### Changed

- History opens only prompt for track-splitting differences, generated marker settings are reapplied from the current settings automatically.

### Fixed

- Loading a recording from History now respects the current generated marker settings, so disabled slip markers no longer reappear from stored history data.

## 0.3.0 - 2026-06-29

### Added

- Add support for NavIC and QZSS constellations.

### Changed

- Avoid showing features for constellations that are not in the loaded data

## 0.2.0 - 2026-06-25

### Added

- Satellite utilization rate plot.
- Loss-of-lock (slip) rate plot: above-mask satellites that drop out or whose SNR falls sharply between epochs. Tunable mask, SNR-drop threshold, and window.
- Slip markers on the map, showing each slipped satellite's before/after elevation, azimuth, and SNR.
- Per-type detection toggles and per-type show/hide for generated markers.

### Changed

- Moved the derived satellite-analysis algorithms into a `gt-analysis` crate.

### Fixed

- The time-range filter now also hides filtered-out points from map hover and click.

## 0.1.0 - 2026-06-21

Initial release
