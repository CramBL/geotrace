# Changelog

## Unreleased

### Changed

- **Interface:** The side panel's "Shelve filtered data…" button acts on the tracks that the filter excludes, and is grayed out while every track passes the filter, while every excluded track sits outside the recording history, or while the session is read-only.
- **Interface:** A track row's context menu, in the tree and in the Visible section, offers "Shelve…" for that track and "Shelve selected…" from two selected tracks on. Both are grayed out while the tracks sit outside the recording history or the session is read-only.
- **Interface:** The confirmation that button raises is titled "Shelve N items?". Ticking its "Delete permanently from history" tickbox changes the title to "Delete N items permanently?" and the button to "Delete permanently".
- **Interface:** History now says "shelved" for a track removed from a recording without deleting its data: the count on a recording row, the "Delete shelved data…" button and its confirmation.
- **Interface:** A track now keeps its number when an earlier track of the same recording is deleted permanently.

### Fixed

- **Interface:** Fixed a second permanent track delete in one session deleting a track the user chose to keep.
- **Interface:** Fixed a permanent track delete announcing a track it had not deleted, when the recording in history no longer had that track: it now reports the failure, and a delete of several tracks with one of them already gone leaves all of them in place.
- **Interface:** Fixed a permanent track delete removing the logs stored with the recording.
- **Interface:** Fixed a permanent track delete removing the whole recording when another stored recording held the points the delete left.
- **Interface:** Fixed shelving a track or deleting one permanently failing on a recording that a permanent track delete or "Delete shelved data" had already re-encoded.

## 0.15.0 - 2026-09-04

### Added

- **Log Viewer:** The line table now colors each line's service name and its error, warning or debug level - a `[WARN]`, an `INFO:`, an upper-case `INFO` before its target, an `<info>`, and any of these past a timestamp the service wrote itself - each switched by its own tickbox in the filter row, "Colour services" and "Colour levels".
- **Log Viewer:** A log whose month abbreviations are written in lower case (`sep 03 21:11:29`) now loads.

### Changed

- **Interface:** The remove confirmation now scrolls its list of items inside the dialog when the remove takes more than ten.
- **Interface:** The update prompt now puts its buttons at the bottom right, like every other dialog.
- **Log Viewer:** The line table now draws each line's date and seconds in a quiet color and its hour and minute brighter where the clock moved on from the line above, and opens each new UTC day with a divider row showing the date.
- **Map & Tracks:** The map now stacks the hover text of everything under the pointer, the topmost layer's first, and shows the snap-to-road hover text in the map's bottom-left corner whenever the pointer is on a snapped track.

### Fixed

- **Interface:** Fixed the force quit confirmation vanishing when the last background write finishes: it now stays up saying the work finished, with the force quit grayed out and a Close button, and closes itself four seconds later.
- **Interface:** Fixed the "Delete hidden data?" confirmation vanishing when the last hidden track goes from the recording list: it now stays up saying that no track is hidden any more, with the delete grayed out and a Close button counting down.
- **Interface:** Fixed the force quit confirmation closing itself with nothing on screen counting down to it: its Close button now counts the seconds down, and the count holds while the pointer is over the confirmation.
- **Interface:** Fixed the loading overlay growing over the map's layer and display toggles while a batch of recordings loads: it now lists four loads and counts the rest, and a press over it reaches the control under it.
- **Interface:** Fixed the History window growing with its listing until it filled the screen: it now keeps the height it is at and scrolls its rows inside it, and it shrinks to a listing shorter than that.
- **Interface:** Fixed the auto-prune confirmation and the "Track settings differ" prompt moving their buttons while they are open.
- **Interface:** Fixed a short dialog opening with a gap between its text and its buttons.
- **Interface:** Fixed the wait for a data directory another GeoTrace is using, and its take-over confirmation, moving their buttons while the other GeoTrace reports on itself.
- **Interface:** Fixed the remove confirmation moving its controls when a log stored with a recording it removes finishes loading.
- **Map & Tracks:** Fixed a track row's hover text drawing over the hover text of the row's snap trigger, coordinate warning icon or hidden snapped track hint.
- **Map & Tracks:** Fixed the map drawing hover text under an open popup: every layer's hover text, and the snap-to-road hover text in the corner, now stay hidden while the popup listing overlapping elements or the right-click menu is open.

## 0.14.0 - 2026-09-03

### Added

- **Interface:** A query's `table` stage now takes an aggregate column, such as `table max(@accel.x)`, and a match lists the channel samples its aggregate columns reduced.
- **Interface:** The time range filter now works below a second: it draws a bar for a recording shorter than a second, its handles select to the millisecond, and a duration under a second reads in tenths, from "0.1s" to "0.9s".
- **Log Viewer:** The log viewer now lists every loaded log grouped by the recording it takes its positions from, each with its own map visibility toggle, and offers loading a listed log's recording. It also lists the logs a loaded recording stores that are not loaded, each with a "Load" button, and the history window counts the logs each recording stores and opens one by name.
- **Log Viewer:** With several logs loaded, a log hexagon's hover text states its log, and clicking a hexagon opens the viewer on that log, scrolled to the hexagon's first line with its lines marked.
- **Map & Tracks:** The side panel now lists the tracks toggled on above the tree, grouped by recording, in an area resized by dragging its divider and kept at that height across restarts. A listed track can be hidden, hovered, revealed in the tree and centered on the map. The list and the tree show track number, distance and duration in the same columns under one header row.
- **Map & Tracks:** A recording with a fix whose latitude or longitude is outside the valid range now loads. The map draws that fix between the fixes around it in the warning color, the side panel shows a warning glyph on its track, and the `invalid_coordinates` query metric counts such fixes.
- **Map & Tracks:** The window for a clicked fix and the hover text of a query results row now show the coordinates the receiver recorded, marking one outside the valid range, and where the map draws a dead-reckoned fix.
- **Map & Tracks:** A recording in which no fix has a valid position now loads, with its tracks drawn nowhere on the map.
- **Map & Tracks:** Hovering or clicking a dead-reckoned fix on the map now selects it.
- **Map & Tracks:** The map warns when a recording has fixes outside the map projection (past 85° latitude).
- **Map & Tracks:** The data quality warnings now list what the app changed in a recording: satellites merged from several rows of one report, SNR readings of 99 dB-Hz discarded as no measurement, event marker styles replaced, and sensor channels whose sample timestamps step backwards. The plot marks those backward steps along its bottom edge, and hovering a mark lists the channels and their two timestamps.

### Changed

- **Interface:** A query using `avg`, `min` or `max` on `lon`, or on a sensor channel that declares a wrap period, is now rejected as ambiguous.
- **Interface:** The time range bar now sets a window of a day or more, and the active range bar, shown only for a recording spanning more than five days, sets one down to a second. Both state this on hover.
- **Log Viewer:** A log now belongs to one recording. Opening that recording again loads its attached logs without opening the viewer, and the toolbar's log button shows an amber count of the logs loaded this way since the viewer was last open. Removing or unloading the recording unloads its logs, and the remove dialog states how many.
- **Log Viewer:** Opening, dropping or pasting a log whose text is already loaded now selects the loaded log, and attaching a log to a recording that already holds it reuses the stored attachment.
- **Map & Tracks:** A backward time step in a recording's fixes at least as long as the split gap now starts a new track.
- **Map & Tracks:** A recording with an event marker style with an unknown icon, or a color that is not a #RRGGBB value, now loads with that marker drawn as a pin or in gray, and the recorded value named in the data quality warnings. A recording with marker or event marker text that is not valid UTF-8 is rejected with an error stating the field.

### Fixed

- **Interface:** Fixed a rate a query computes, such as `velocity / eph`, comparing 60 times too small against a rate literal.
- **Interface:** Fixed a query's `spread`, `std` and `delta` of `lon` measuring nearly a full turn across the antimeridian, and of a sensor channel ignoring its declared wrap period.
- **Interface:** Fixed a query's `min`, `max` and `spread` of a sensor channel ignoring a sample that is not a number, and a query aggregate over a channel whose samples are out of time order reading the wrong samples.
- **Interface:** Fixed queries over a recording faster than 1 Hz: a duration window covered every fix of a second, `accel` was absent between two fixes of the same second, a window's channel aggregate read the samples before its first fix, and a sensor channel query drew its match halo on the wrong fixes.
- **Interface:** Fixed queries across a backward time step: a duration window mixed the fixes on both sides of the step into one match, a window's channel aggregate dropped the samples between its fixes, and the time range filter dropped the fixes after the step.
- **Interface:** Fixed a sensor channel query's `keep` leaving a track with no match fully drawn on the map.
- **Interface:** Fixed the query results not graying out while the query window is closed, or when a recording of the same file name replaces the one the query ran over.
- **Interface:** Fixed a query's `sys_time` and `clock_delta` dropping the microseconds of a host timestamp.
- **Interface:** Fixed the time range and track filters not applying everywhere. A sensor channel query, the query results' map buttons, the snapped track and its error whiskers, the log hexagons, the plot's extent and chips, and the highlight of the plot cursor and of a hovered results row now leave out what the filter hides.
- **Interface:** Fixed the time range filter's bar vanishing for a recording whose clock steps backwards, and spanning decades when a recording with no fixes is loaded beside it.
- **Interface:** Fixed a drag of an active range bar handle to its end dropping the filter's bound, and a drag past the other handle moving that handle.
- **Interface:** Fixed the confirmations for deleting archived days and for force-quitting moving their buttons while they are open, and a press aimed at the force-quit Cancel reaching the shutdown window behind it.
- **Log Viewer:** Fixed the association dialog moving its rows when the history database reports that the chosen recording already holds the log.
- **Map & Tracks:** Fixed a track that crosses the antimeridian, circles a pole or reaches past 85° latitude: the min spread filter understated its spread, its bounding box covered an arbitrary arc of longitudes, zooming the map to it missed, a dead-reckoned fix or a log entry between two fixes was placed half a world away, and a fix past 85° was drawn far off the map.
- **Map & Tracks:** Fixed where a dead-reckoned fix is drawn: on the preceding fix in a recording faster than 1 Hz, an event marker on a dead-reckoned stretch drawn away from the track, and a track's length, spread and bounding box measured at the fix's recorded coordinates.
- **Map & Tracks:** Fixed a recording whose clock steps backwards dropping markers and sensor samples inside a track, reporting a negative recorded time, and showing an inverted time range in the History window.
- **Map & Tracks:** Fixed the plot drawing a sensor channel whose sample timestamps step backwards as one line through every step, and its shared y-axis fitting the clock offset of fixes recorded before the device's clock was corrected.
- **Map & Tracks:** Fixed the plot drawing one fix past each edge of the time range filter, a line across a filter window with no fix, and leaving its lines unchanged after a small move of the filter's end.
- **Map & Tracks:** Fixed a heading swing between two northward readings disappearing from the plot when zoomed out, and the same for a sensor channel that declares a wrap period.
- **Map & Tracks:** Fixed an SNR of 99 dB-Hz, which firmware writes for no measurement, counting as a signal strength, and a satellite listed on two rows of one report counting twice in the satellite counts, lost-lock slips and utilization rate.
- **Map & Tracks:** Fixed the plot's show/hide-all button leaving the sensor channel lines and the solar flare markers as they were.
- **Map & Tracks:** Fixed the sky trails plot shifting when the satellite counts beside it change width, and the window for a clicked fix drawing its content over its title bar when scrolled.

## 0.13.0 - 2026-08-25

### Added

- **Environment Data:** Added support for Geomagnetic indices (Kp/Hp30) and Ionosphere maps (TEC), including auto-downloads, map heatmaps, and query support.
- **Environment Data:** Added solar flare markers from NASA's DONKI catalog to the plot. Hovering a marker or the solar flare chip shades the flare's duration on the view.
- **Environment Data:** Added warnings (via toast and map icon) when space weather or aircraft interference reaches levels that could disrupt satellite navigation. Warnings detail the affected tracks and metric values, and TEC deviations state storm durations and preceding geomagnetic activity.
- **Log Viewer:** Added a dedicated log window to inspect loaded logs, view parse summaries, and center the map by clicking a line.
- **Log Viewer:** Added plain text and regex filtering with the ability to save filters. Filtered lines now display on the map as color-coded hexagons with two-way hover sync.
- **Log Viewer:** Added the ability to attach logs to specific recordings on load and to paste log text directly using `Ctrl+V`.
- **Map & Tracks:** Added a right-click option to snap all tracks in a recording to the road at once. Finished query matches now flare up and settle on the map to stand out, and tooltips identify which recording a point belongs to.
- **Settings & Storage:** Added a storage manager to view disk usage for environment data and auto-prune old days. Also added a search field and a live preview for the recording name template.
- **System:** Added the `--offline` flag to run without network access.
- **System:** Closing GeoTrace now finishes writes in progress, with a shutdown window listing the progress of unfinished writes and archive deletes.
- **System:** Only one GeoTrace writes to a data directory at a time to prevent conflicts. Additional instances will wait until the directory is free, or can be started in a "read-only" mode that safely reads recordings and archives alongside the active window.

### Changed

- **Environment Data:** Grouped space weather and interference chips together with descriptive hover states.
- **Interface:** Redesigned query results into a tabbed panel below the editor, featuring a sortable table, visual size bars, and a copy button for tab-separated values. You can drag the splitter to divide the tab or move the match list to its own window.
- **Interface:** The Settings window now displays one category at a time using a left-side navigation rail.
- **Interface:** A recording's hover text and details dialog now state the time range it covers beside the recorded time its tracks hold.
- **Interface:** Non-essential label text no longer selects so the cursor remains ordinary, while text worth copying (metadata, log lines, query examples) remains selectable.
- **Log Viewer:** Log files now successfully load even if some lines have unrecognized timestamps by interpolating from neighboring data.
- **System:** The window now opens right away and databases open behind it, so a large archive no longer delays the first frame.

### Fixed

- **Interface:** Every window now stays inside the screen and scrolls its content when there is more of it than fits. Buttons in prompts are now grouped bottom-right.
- **Interface:** Fixed external links properly opening in the browser, hover states correctly highlighting without overlapping, and the file legend no longer covering the settings and query windows.
- **Map & Tracks:** Fixed snapped tracks improperly routing through nearby roads during dead-reckoning gaps (e.g., parking garages) and eliminated flickering on overlapping tracks.
- **Settings & Storage:** Older environment archives are now automatically rebuilt on load so that deleting old days properly frees disk space.

## 0.12.0 - 2026-08-15

### Changed

- Updated egui to 0.36 (along with egui_plot, egui_kittest and wgpu 30).

### Added

- A clock offset excursion - one sample whose GPS/system offset jumps away and straight back, as when a receiver resumes after a recording gap - no longer flattens the plot's y-axis. It is marked at the edge of the view with its real offset on hover, and on the map. Threshold and on/off are in Settings.

### Fixed

- Hovering a shortened name shows one tooltip.
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

- History elements show a cursor that matches what they do.
- Double-clicking a History identity always opens the rename editor now.

## 0.9.0 - 2026-07-23

All optimizations, no features, no fixes.

### Changed

- Map markers, navigation arrows and fix-loss chevrons are now vector meshes pre-tessellated at build time and drawn with GPU instancing. They stay sharp at every zoom level and on high-DPI screens, and dense recordings render them in one instanced draw call per icon type.
- Reduced per-frame allocation and hashing in the map's overlay placement pass (satellite labels, sky glyphs, fix icons): reused scratch buffers, a faster hasher for the decimation grids, stack-allocated vectors for fixed-size temporaries, and the GPU icon instance transform computed once per instance.
- Clip the track plot's masked-fix and unsnapped-point markers to the visible x-range before drawing and hit-testing them.
- Cache the display settings popup's per-category counts, recomputing only when an input changes.

## 0.8.0 - 2026-07-22

### Added

- A trail opacity control in the Sky trails window, to quieten the paths on a busy multi-constellation sky or turn them up.
- Scrubbing the Sky trails window now marks the current time on the track plot, the same line a track-point hover draws, so playback follows along there too.

### Changed

- Sky trails now draw as a comet's tail: brightest at each satellite's current position and fading back over the path it has travelled, so the direction of travel reads at a glance.
- "In fix only" in the Sky trails window now trims each trail to the parts where the satellite was actually used in the fix.

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
  Nothing is uploaded without consent.
  The server and matcher options are configurable in Settings (default: the public FOSSGIS instance).
- A File menu with Open and an About dialog.
- A "Recording name" template in Settings controls how recordings are labelled in the side panel (`{title}`, `{device}`, `{identity}`, `{filename}`).
- Recording metadata (title, device, notes, travel mode) shows behind a note icon in the side panel and the History window.
- Recording identities can be renamed in the History window: double-click the name, or right-click for Rename.

### Changed

- History window columns are resizable, and the identity column gets the spare width.
- Distance and duration in the side panel carry a road and clock icon.
- Query summaries name tracks that carry no values for a referenced metric.

### Fixed

- Long names truncate in the side panel, the History window, and related dialogs, with the full value on hover.
- Light mode: status colors, plot lines, and query syntax highlighting are readable now, and the plot gets a faint-grey canvas.
- Focusing a track dims the map with a gentle veil.

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
- Satellite-count labels are now placed at diagnostically relevant points (fix-quality changes, signal dips, recoveries after loss), and no longer shuffle while panning the map.

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
