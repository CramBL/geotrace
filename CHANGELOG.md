# Changelog

## Unreleased

### Added

- Picking a mode under "Snap again as" that the track already has snap to road data for now asks whether to replace it.
  The snap status icon offers the same menu on right-click.
- Right-clicking a recording offers "Snap again as" for its tracks at once, asking whether to snap the selected tracks or all of them.
- With more than one file loaded, a fix point's hover tooltip, the stacked label shown when map points overlap and the clicked-point window each name the recording the point belongs to.
- Editing the recording name template in Settings opens a guide below the field, listing the tokens and previewing the template as you type.
- A recording name template token can cap its length, as `{title:12}` for at most the first 12 characters of the title.
- Loading a recording downloads the Kp and Hp30 geomagnetic indices for the days it spans into a local archive, so it can be read against what the geomagnetic field was doing while it was made. The host serving them is in Settings under "Geomagnetic indices", and nothing is downloaded in offline mode.
- Settings' "Geomagnetic indices" section shows what the index download is doing, how many days of the loaded recordings are archived, and the days that failed with their cause. It downloads index history for a date range too, skipping days already archived, with progress and a cancel button.
- Both history downloads, interference and geomagnetic indices, are grayed out in offline mode with hover text saying so.
- The plot draws the "Hp30 index" line, one value per fix from the archived period the fix falls in, with the three-hourly "Kp index" one chip away. Both chips stay grayed out until the days a recording spans are archived.
- Hovering an index line names the value, its storm class on the G scale, and the period it covers. The indices are queryable as `hp30` and `kp` too, so `where hp30 > 5 and slip_all > 2 per min` selects the fixes that lost lock during a geomagnetic storm.
- Loading a recording downloads JPL's global ionosphere maps for the days it spans into a local archive, so the ionospheric delay it was recorded under is on hand. The host serving them is in Settings under "Ionospheric TEC", and nothing is downloaded in offline mode.
- Settings' "Ionospheric TEC" section takes an ordered list of mirrors instead of a single host, tried in turn until one has the day's map file.
  A day no mirror could serve names each of them and the status it returned.
- The `--offline` flag runs GeoTrace without network access: no map tiles, downloads, snapping, or update check. It replaces the undocumented `GEOTRACE_OFFLINE` environment variable, which is no longer read.
- Settings' "Aircraft interference" page shows what the interference download is doing, how many days of the loaded recordings are archived, and the days that failed with their cause.
- Settings' "Ionospheric TEC" page downloads map history for a date range, skipping days already archived, with progress and a cancel button. It also shows what the map download is doing and how many days of the loaded recordings are archived.
- Auto-storing recordings and auto-pruning are adjustable on Settings' "Application" page as well as in the History window.
- Settings' "Interface" page holds the theme, the map layer and the Mapbox token, alongside the top bar and map controls that already set them. A "Test" button next to the token fetches one satellite tile with it and reports what the host answered.

### Changed

- The settings window shows one category at a time, picked from a rail on the left, instead of every section stacked in one column. The recording name template moved to the "Interface" page and the update check to "Application".
- A log file loads even when lines in it carry no recognised timestamp: those lines are skipped and the rest are kept. The load fails only when no line at all parses, and then names the first line it could not read.

### Fixed

- Hover labels no longer stack on top of each other, in the plot and on the map's interference layer.
- The plot's file legend no longer covers the settings and query windows.
- The snapped track no longer fans out across nearby roads where the receiver was dead reckoning, as in a parking garage.
  Ghost fixes are never sent to the map-matching server, and the snapped track breaks at the gap instead of being routed through it.

## 0.12.0 - 2026-08-15

### Changed

- Updated egui to 0.36 (along with egui_plot, egui_kittest and wgpu 30).

### Added

- A clock offset excursion - one sample whose GPS/system offset jumps away and straight back, as when a receiver resumes after a recording gap - no longer flattens the plot's y-axis. It is marked at the edge of the view with its real offset on hover, and on the map. Threshold and on/off are in Settings.

### Fixed

- Hovering a shortened name shows one tooltip instead of two stacked ones.
- Use the recording name template in the rest of the app: the plot's file legend, line names and hover labels, the map's right-click menu and the remove-confirmation dialog all show the same name as the side panel.
- The plot hover label no longer starts with a blank line when the cursor is not on a line.

## 0.11.0 - 2026-08-05

### Added

- An aircraft interference map layer: where aircraft reported degraded GNSS navigation, from gpsjam.org's daily data over adsbexchange.com's reports. Toggle it in the eye popup, step the day it shows, and hover a cell for the counts behind its colour. Days are archived on disk as they are fetched, so a day is downloaded once and stays available offline.
- An "Aircraft interference (%)" plot metric, valued per fix from that fix's own UTC day, breaking the line where no day is archived. It is queryable too, so `where jamming > 10 %` selects the stretches under reported interference.
- Download interference history for a date range from settings, so old recordings have data the moment you open them. Days already archived are skipped, progress is shown, and it can be cancelled.

### Fixed

- A pinned point popup no longer stays open once the map stops drawing that point - a query hiding it, the time filter excluding it, or its track being switched off. The pin is kept, so the popup comes back when the point does. Point rows in the side panel and the query results table also stop pinning points that are not on the map.

## 0.10.0 - 2026-07-28

### Added

- `geotrace --update` updates an installed build in place from the terminal, the same update the app offers on startup but without opening a window.
- Sort the History window by any column, clicking a header again to reverse it.
- Hovering a History row breaks down what the recording holds, down to its ad-hoc sensor channels.

### Fixed

- History elements show a cursor that matches what they do, rather than a text-editing one on everything.
- Double-clicking a History identity always opens the rename editor now, instead of sometimes selecting a word.

## 0.9.0 - 2026-07-23

All optimizations, no features, no fixes.

### Changed

- Map markers, navigation arrows and fix-loss chevrons are now vector meshes pre-tessellated at build time and drawn with GPU instancing, instead of pre-rasterised bitmaps. They stay sharp at every zoom level and on high-DPI screens, and dense recordings render them in one instanced draw call per icon type rather than one textured quad each.
- Reduced per-frame allocation and hashing in the map's overlay placement pass (satellite labels, sky glyphs, fix icons): reused scratch buffers instead of reallocating, a faster hasher for the decimation grids, stack-allocated vectors for fixed-size temporaries, and the GPU icon instance transform computed once per instance.
- Clip the track plot's masked-fix and unsnapped-point markers to the visible x-range before drawing and hit-testing them, instead of processing the whole track each frame.
- Cache the display settings popup's per-category counts, recomputing only when an input changes instead of walking all loaded points each frame.

## 0.8.0 - 2026-07-22

### Added

- A trail opacity control in the Sky trails window, to quieten the paths on a busy multi-constellation sky or turn them up.
- Scrubbing the Sky trails window now marks the current time on the track plot, the same line a track-point hover draws, so playback follows along there too.

### Changed

- Sky trails now draw as a comet's tail: brightest at each satellite's current position and fading back over the path it has travelled, so the direction of travel reads at a glance.
- "In fix only" in the Sky trails window now trims each trail to the parts where the satellite was actually used in the fix, rather than blinking whole trails on and off during playback.

### Fixed

- Filtering on channel data like `@accel.x` without a window no longer sends you in circles - the hint now points to a query that actually works.

## 0.7.0 - 2026-07-21

### Added

- Sky plot: see the satellites used in each fix - a polar plot when you hover or click a track point, plus a toggleable ring or disc overlay showing directions along the whole track.
- Sky trails window: replay every satellite's path across the sky over the whole recording, scrubbing through time with the map following along. Live per-constellation stats and cycle slips, a signal-strength heatmap of the current fix, and toggles to hide the trails for a snapshot view or keep only the satellites in the fix.

### Changed

- The track point window resizes and repacks to handle busy multi-constellation fixes, keeping the sky plot beside the satellite tables. Folds are remembered.

### Fixed

- The History window resizes smoothly and can be shrunk again, its filter field no longer overlaps the toolbar in a narrow window.
- Focusing a track now darkens the map in light mode too.

## 0.6.0 - 2026-07-16

### Added

- Snap to road: match a track against the OpenStreetMap road network, from its side-panel action or automatically (opt-in).
  The match draws as a dashed line on the map with road details on hover, the "Snap error (m)" plot metric shows each point's distance to the road next to EPH, and `snap_error` is usable in queries.
  Results are kept in history, marked stale when settings change, and can be re-run as another travel mode.
  Nothing is uploaded without consent; the server and matcher options are configurable in Settings (default: the public FOSSGIS instance).
- A File menu with Open and an About dialog.
- A "Recording name" template in Settings controls how recordings are labelled in the side panel (`{title}`, `{device}`, `{identity}`, `{filename}`).
- Recording metadata (title, device, notes, travel mode) shows behind a note icon in the side panel and the History window.
- Recording identities can be renamed in the History window: double-click the name, or right-click for Rename.

### Changed

- History window columns are resizable, and the identity column gets the spare width.
- Distance and duration in the side panel carry a road and clock icon.
- Query summaries name tracks that carry no values for a referenced metric, instead of only counting skipped points.

### Fixed

- Long names truncate (full value on hover) instead of stretching the side panel, the History window, and related dialogs.
- Light mode: status colors, plot lines, and query syntax highlighting are readable now, and the plot gets a faint-grey canvas.
- Focusing a track dims the map with a gentle veil instead of washing it to white.

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
