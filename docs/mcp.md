# GeoTrace MCP guide

Follow the [setup instructions](../README.md#inspect-recordings-from-an-mcp-client) to connect your MCP client to GeoTrace.

## Configure your client

**Copy MCP config** uses the absolute path to the companion in your GeoTrace installation.
If you move GeoTrace or an upgrade changes its install path, copy the config again.
If you build GeoTrace yourself, build `geotrace` and `geotrace-mcp` in the same output directory.

For manual setup, configure your client to launch `geotrace-mcp` as a stdio server.

```json
{
  "mcpServers": {
    "geotrace": {
      "command": "/absolute/path/to/geotrace-mcp"
    }
  }
}
```

On Windows, use the path to `geotrace-mcp.exe` and escape backslashes in JSON.
Stop active MCP sessions before updating GeoTrace on Windows.

### macOS Gatekeeper

GeoTrace releases include `geotrace` and `geotrace-mcp` as unsigned binaries.
If Gatekeeper blocks a binary from the [official release](https://github.com/CramBL/geotrace/releases/latest), remove its quarantine attribute with `xattr`.
Substitute the paths from your installation and use the command for each blocked binary.

```sh
xattr -d com.apple.quarantine "/path/to/geotrace"
xattr -d com.apple.quarantine "/path/to/geotrace-mcp"
```

After a blocked launch, you can also use **Open Anyway** under **System Settings > Privacy & Security**.
See [Apple's instructions](https://support.apple.com/en-us/102445).

## Start and stop MCP

Click **Start MCP** in **Settings > MCP** or launch `geotrace --session-bridge`.
Click **Stop MCP** in **Settings > MCP** to close access to GeoTrace.
GeoTrace clears captured traffic.
GeoTrace continues work from accepted requests.
After restarting MCP, request new loaded recording and track references.
If a query is still running, call `run_query` to get its new job identifier in the `busy` response.

## Tools

| Tool | Use |
|---|---|
| `get_session_state` | Read a summary of the current desktop session. |
| `set_session_view` | Change map layers, category visibility, or the global time filter. |
| `open_panel` | Open or raise a desktop panel. |
| `list_recordings` | List loaded recordings and get their references. |
| `manage_recordings` | Open local recordings or logs, or unload recordings from this session. |
| `get_recording` | Read a recording summary, loaded track references, and stored track rows. |
| `get_track` | Read a track's time range, counts, geometry, and visibility. |
| `get_range_details` | Inspect recorded points by index. |
| `set_track_visibility` | Show, hide, or solo loaded tracks. |
| `list_metrics` | Discover query metrics and channels from loaded recordings. |
| `check_query` | Validate query text and get diagnostics. |
| `open_query` | Open an editable draft in the Query window. |
| `run_query` | Start a query over enabled tracks with the current filter. |
| `get_query_status` | Check a query job's progress or completion summary. |
| `cancel_query` | Cancel a running query. |
| `get_query_results` | Read pages of matches or rows from one match. |
| `show_on_map` | Frame and highlight a track, range, or point-query match. |
| `open_point` | Center the map on a point and open its information. |
| `show_on_plot` | Frame a track, range, or query match on the plot. |
| `get_settings` | Read current Analysis and recording Processing settings. |
| `describe_setting` | Read a setting's value, limits, and help. |
| `open_setting` | Reveal a control in Settings. |
| `set_setting` | Change a setting and get its effective value. |
| `apply_processing` | Rebuild loaded tracks and generated markers using current settings. |
| `list_history_recordings` | List stored recordings and optionally filter by UTC date overlap. |
| `open_history_recording` | Start opening a recording from History. |
| `get_history_open_status` | Check a History open and get the loaded recording reference when ready. |

## Desktop session

Call `get_session_state` without arguments to inspect the running desktop in one request.
The result includes `recording_count` and the first five `recordings` in load order, with names, session references, and the same metadata limits as `list_recordings`.
Pass `next_after` as `after` to `list_recordings` for more loaded recordings if `next_after` is present.

`open_panels` lists which of `history`, `log_viewer`, `plot`, `query`, `settings`, and `sky_trails` are open.
`time_filter` has kind `all`, `empty`, or `bounds`.
The `bounds` kind includes inclusive `start` and `end` UTC timestamps, with null for an unset endpoint.
An endpoint outside RFC 3339 years 0000 through 9999 also appears as null, with `start_unrepresentable` or `end_unrepresentable` set to true.

The `map` section includes the selected base layer and the display categories enabled in the map visibility controls.
`map.bounds` reports the most recently laid out map viewport in degrees, or null when the map is hidden or has no usable viewport.
Its `west_deg` and `east_deg` values use the viewport's longitude interval and can extend beyond ±180°.
`map.pinned_point` uses the `open_point` input shape for a navigation point with a current track reference, or null if there is no such pin.
A filter can hide a point while the pin remains.

`query` reports the latest desktop or MCP query phase, or null if there is no operation.
`query.job` appears as a session-bound job handle when one exists for the caller, or null otherwise.
The `completed` phase records that the run finished.
Use `get_query_status` with the job handle to check result freshness.

Pass one of the names listed for `open_panels` to `open_panel`, for example `{"panel":"query"}`.
For `sky_trails`, include a current `track` reference from `get_recording`.
GeoTrace checks that the track has satellite azimuth and elevation reports before opening Sky trails.
Supply only `panel` for the other panels.
GeoTrace returns `panel_open_queued` after recording the panel choice.
GeoTrace brings a floating panel to the front after layout.
For Plot, it makes the docked pane visible.
Opening Query preserves its draft and results.
Opening Settings keeps its current page.

Use `set_session_view` to update the desktop view and return the same shape as `get_session_state`.
Omitted fields retain their current values.
Set `base_layer` to `open_street_map` or `satellite`.
The satellite layer requires a Mapbox token configured in Settings.
Each entry in `display_categories` sets one map category's visibility.
Specify each category at most once, using the names in the tool schema.
`map.visible_categories` lists the categories currently enabled.
Base layers and category visibility affect what the map draws.
Track checkboxes and query eligibility stay the same when either of those changes.

```json
{
  "base_layer": "open_street_map",
  "display_categories": [{"category":"satellite_labels","visible":false}],
  "time_filter": {"action":"set","start":"2026-02-01T14:00:00.123456789Z"}
}
```

Use `time_filter` with action `set` and at least one of `start` and `end`.
The inclusive bounds use RFC 3339 UTC timestamps ending in `Z` and accept up to nine fractional digits.
Omit an endpoint to leave that side unbounded.
Use `{"time_filter":{"action":"clear"}}` to clear the time constraint.
Other global filters stay in effect.
GeoTrace rejects the whole update if any field is invalid.
Changing the time constraint can make a completed query stale.
Use `get_query_status` to check its freshness.

## Recordings

Use `manage_recordings` with action `open` and a `paths` array of 1 to 16 absolute local paths on the computer running GeoTrace.
Use `.gtd` for recordings or `.log` for logs.
File extensions are case-insensitive.
On Windows, use local drive paths and escape backslashes in JSON.
The encoded request must fit within 4 KiB.

```json
{"action":"open","paths":["/data/survey.gtd","/data/receiver.log"]}
```

GeoTrace checks each path's form before it queues the batch and counts identical path strings once.
The `files_queued` reply gives the number of queued paths.
Use `list_recordings` to check when recordings are loaded.
Open the Log viewer to see loaded logs.
GeoTrace shows load failures and requests for History or log association decisions in the same desktop controls as a file drop.

Use action `unload` with a `recordings` array of 1 to 16 IDs from the current session's `list_recordings` or `get_session_state`.
GeoTrace validates every reference before unloading and counts repeated references once.
The `recordings_unloaded` reply gives the number removed from the session.
The original files and stored History recordings stay available.

The `list_recordings` tool lists the recordings that are currently loaded.
It accepts an optional `limit` and an `after` cursor.
Each page contains at most 100 recordings, which is the default limit.
Use the returned `next_after` as `after` to continue, and stop when `next_after` is null.

Pass a recording's returned `id` as `recording` to `get_recording`.
It returns the recording summary and up to 100 loaded track references in loaded order.
Recording summaries set `metadata_truncated` to true when the bridge shortens text metadata for the reply.
If either recording time bound falls outside RFC 3339 years 0000 through 9999, `time_range` is null and `time_range_unrepresentable` is true.
Pass its `next_after` as `after` to continue.
Match each `track_states` entry to its track reference by `index`.
Pass a track reference as `track` to `get_track` for its recorded time range and counts, or to `get_range_details` for points in recorded order.
`get_recording` and `get_track` report the track's checkbox and row selection as `tree.enabled` and `tree.selected`.
These values describe the desktop track tree independently of time filters and map display categories.
`stored_state` reports `live` or `shelved` when GeoTrace has a current History membership observation for the track.
A null `tree` or `stored_state` marks unavailable state.

`get_recording` also returns `stored_tracks` with a separate page in one-based stored track number order.
The stored rows include `live`, `shelved`, and `deleted` states, including shelved tracks outside the loaded set.
Stored row numbers are separate from loaded track `index` values and cannot be passed as track references.
Pass `stored_tracks.next_start` as `stored_track_start` to continue these pages independently of `after`.
Stop when `stored_tracks.next_start` is null.
The requested `limit` applies to each page, with a maximum of 100 rows per page.
`stored_tracks` is null when GeoTrace lacks a current checked History observation.
A storage change can invalidate that observation until GeoTrace obtains a new one.

`get_track` reports distance and bounds from the map geometry when available.
Longitude bounds extend east from `longitude_start_deg` by `longitude_span_deg`, including across the antimeridian.
`get_range_details` accepts a zero-based inclusive `start` and exclusive `end`.
Use `next_start` as `start` with the same `end` to continue through pages of up to 100 points.
The tool reports exact UTC timestamps in recorded order, including across clock reversals.
The source for `time` is the receiver clock when available and the host clock otherwise.
Missing recorded coordinates appear as null.
The `eph_m` and `velocity_mps` fields use status objects such as `{"status":"finite","value":2.5}`.
An unreported measurement has status `missing` and a non-finite measurement has status `non_finite`.
Only a finite measurement includes `value`.

Use recording IDs and cursors from the current desktop session.
Start again with `list_recordings` after restarting GeoTrace.
Get fresh track references with `get_recording` after changing the loaded recordings.

Use `set_track_visibility` with an `action` and a `tracks` array of 1–24 loaded track references.
`show` enables the listed tracks and `hide` disables them.
`solo` enables the listed tracks and disables every other loaded track.
GeoTrace checks all references before it applies the action and counts duplicate references once.
`targeted_tracks` counts distinct requested tracks.
`changed_tracks` counts track checkbox changes.
The change takes effect in the desktop immediately and applies to the enabled-track scope of `run_query`.
GeoTrace saves track visibility through the same History process as desktop checkbox edits when a checked layout and saved display settings are available.
The reply confirms the in-memory change.

## Queries

Read the MCP resource `geotrace://query-language` for syntax, operators, units, and examples.
The companion advertises it through `resources/list` and serves it through `resources/read`, even when GeoTrace is closed.

`list_metrics` returns the built-in query identifiers, summaries, quantities, accepted unit examples, and sources.
Metric definitions describe where values can come from, including environment archives and completed road snapping runs.
Actual values depend on the recording and time range.
The tool also lists channels from the current recordings, with units, wrap periods in degrees, and component labels.
The `reference` fields contain query expressions such as `@orientation.x`, or null for labels outside the query syntax.
When a component label repeats, its query reference selects the first column with that label.
The query checker rejects channels with conflicting definitions across loaded tracks.
The channel entry includes the total number of conflicts and up to eight diagnostic messages.
Entries with oversized channel metadata have status `metadata_omitted` and count toward the page limit.
Set `channel_limit` to request fewer than 100 channels per page.
Pass `next_channel_start` as `channel_start` for the next page.
Restart channel pagination after changing the loaded recordings.

Pass query editor text as `text` to `check_query`.
Use blank lines to separate queries, and check a query with a channel source on its own.
The result has status `valid`, `invalid`, or `empty` for text with only comments or whitespace.
Invalid results include messages, optional hints, and spans with zero-based UTF-8 byte offsets into the full submitted text.
Each span includes its start and excludes its end, with equal offsets for an insertion point.
The serialized request must fit in 4 KiB.
Check fewer query blocks at a time if the request or its diagnostics exceed the bridge limits.
For a single diagnostic that exceeds the reply limit, check that query in the GeoTrace editor.

Pass `text` to `open_query` to replace the draft in the Query window.
The tool accepts any draft that fits within the bridge request limit, including text with syntax errors.
After `query_draft_queued`, use the editor to revise the draft and run it when ready.

Submit the same `text` to `run_query` to start a background query job.
GeoTrace evaluates the tracks enabled in the track tree and applies the current global filter, including its time window.
GeoTrace opens the Query window with the accepted text and adds the run to query history.
The window and `get_query_results` show the same completed run.
A start request returns status `busy` and the active job identifier if a desktop or MCP run is in progress.
When GeoTrace can start a job, invalid or empty text returns the same validation result as `check_query`.

Pass the returned `job` to `get_query_status` or `cancel_query`.
A successful start returns the registered job identifier and the latest status.
The companion polls within a one-second budget that starts with the request.
When the job completes in time, `run_query` returns `completed` with the query summaries.
If the budget expires or a status request fails, the companion returns the latest status it received for that job.
If the connection closes before the reply arrives, retry `run_query` to get the active identifier or start a new job.
Poll a pending job until the status is `completed`, `cancelled`, `failed`, or `stale`.
Completion reports the match, track, and skipped counts for each query block in input order.
The status reports `stale` when the current recording data, filter, enabled tracks, or derived inputs differ from the run's inputs.
The Query window's Cancel button cancels the same job as `cancel_query`.
After a cancellation, the window can still display the previous run.
GeoTrace retains a completed job until another run starts, the query filter is cleared, or the desktop session ends.
Editing the draft leaves the completed job available.

For a completed job, pass its `job`, the zero-based `query` block index, and a `page` of `{"kind":"matches"}` to `get_query_results`.
The match page lists each match's track reference, half-open range, first and last timestamps, and aggregate values.
The match position uses zero-based ordinals for the matching track group and the match within that group.
Pass its `next_start` as `page.start` to continue with the same job and query.
To read the table rows of one match, set `page.kind` to `rows` and pass the match's `position` as `page.matched`.
Omit `page.start` for the first row, then pass the row page's `next_start` as `page.start` to continue.
A null `next_start` marks the final page.
Point indices refer to the original track, and point times use RFC 3339 UTC.
Channel indices refer to the filtered sample timeline, and sample times use Unix seconds.
Column ordinals give the table order across match aggregates and row values.
Point metrics and aggregates use evaluator units, with ratios expressed as fractions and the `time` metric in Unix seconds.
Channel components use the track's declared unit.
Cells have status `finite` with a numeric value, `missing`, or `non_finite`.
Both page kinds return at most 100 entries and 2,048 values.
Column metadata that exceeds 64 KiB causes a size error.

## Map and plot

Pass a `target` with `kind` set to `track` and a checked `track` reference to `show_on_map` to frame and highlight its visible path.
Set `target.start` and `target.end` for a half-open range of original point indices, or omit them for the whole track.
To frame a retained point-query match, pass a `target` with `kind` set to `query_match`, the same `job` and `query` used in `get_query_results`, and the match's `position` as `target.matched`.
Channel sample ranges produce an error.
Enable Query highlights in the map's visibility controls before calling `show_on_map`.
GeoTrace returns an error if Query highlights is disabled.
Pass a `track` and a zero-based `point` index to `open_point` to center the map and open point information.
Both tools use the current filters and visibility and report an error for a target with no visible geometry.
The `map_presentation_queued` status acknowledges the request.
GeoTrace applies the request when the map next draws if the recording, filter, query run, and target visibility still permit it.
A later camera request supersedes a queued request.
A later pin change supersedes a queued point request.
GeoTrace discards a queued range request if Query highlights is disabled when the map draws.
Use Query highlights to hide or show an active range.
GeoTrace clears a `show_on_map` match highlight when its job becomes stale or another job replaces it.
The map shows which point, range, or query match the request presented.
Dismiss removes the request's highlight or point information and keeps the current map view.

Pass the same `target` to `show_on_plot` to open the plot and frame the selected timestamps.
For a channel match, GeoTrace frames the retained sample times and enables the matched channel.
Use the optional `metrics` array to enable plot metrics by their query identifiers, such as `velocity` and `eph`.
GeoTrace reports an error for a query identifier with no corresponding plot metric.
GeoTrace frames visible timestamps even when a selected metric has missing values.
The `plot_presentation_queued` status acknowledges the request.
GeoTrace applies a current request on the next frame with a visible plot area.
With Sync to map enabled, the plot keeps that time range until the map's visible time range changes or the user changes the plot view.
The plot shows which range or query match it framed.
Dismiss removes the presentation label and keeps the current time range and enabled series.

## Settings

Use `get_settings` to list the current values and descriptions for Analysis controls and recording controls on the Processing page.
Pass a setting's `key` to `describe_setting` for its help, value, and limits, or to `open_setting` to reveal the control in Settings.
For numeric settings, the description includes the unit and inclusive bounds.
The `value` field is null if a loaded configuration contains a non-finite number.
An `apply` value of `live` means the plot updates as the control changes.
With `live_with_deferred_markers`, the plot updates as the control changes and GeoTrace regenerates affected markers on load or through Apply to loaded data.
With `deferred_processing`, GeoTrace uses the setting when it processes a recording.
Use Apply to loaded data in Settings to process the current recordings with updated settings.
`open_setting` returns `setting_open_queued` when the bridge schedules the navigation.

Use `set_setting` with a setting's `key` and a boolean or number in its reported unit.
Numeric input must be finite and within the reported bounds.
Enter the track split gap in whole seconds.
Enable clock discontinuity detection before setting jump sensitivity.
GeoTrace returns the effective value in the current session.
GeoTrace saves eligible settings changes automatically and keeps changes in memory in read-only sessions.
The `apply` field indicates when GeoTrace uses the setting.

Use `apply_processing` to rebuild the loaded tracks and generated markers with the current Processing settings, as with Apply to loaded data in Settings.
Send an empty arguments object or omit the arguments.
GeoTrace requires a loaded recording with navigation points.
GeoTrace returns `processing_applied` after rebuilding the loaded tracks and generated markers.
Saving track visibility in History may finish later.
A timeout can occur during a rebuild.
GeoTrace finishes a rebuild that has started even if the client disconnects.
Use `get_recording` to obtain fresh track references and rerun queries against the rebuilt tracks.
History keeps its stored track layout.

## History

Use `list_history_recordings` to discover stored recordings, including recordings that are closed in the app.
GeoTrace lists recordings by the start of their navigation time range, newest first.
Recordings with equal start times can appear in either order.
Set `limit` to 1 to get the first matching recording.
Filter by UTC dates with `dates.from` and `dates.through`, both inclusive.
A recording matches when its navigation time range overlaps the date filter.
Recordings without a stored time range appear at the end of the unfiltered listing.
The `start` and `end` bounds in `time_range` use RFC 3339 UTC strings and preserve microsecond precision.
GeoTrace returns `time_range` as null for a missing range or a bound outside years 0000 through 9999.
For a bound outside those years, GeoTrace also sets `time_range_unrepresentable` to true.
Ordering and date filters use the stored bounds.
Pass `next_cursor` as `cursor` with the same dates to continue, or start again after History refreshes or the app restarts.
Recording IDs remain stable across app restarts with the same History database contents and path.
GeoTrace returns up to 32 recordings per page and sets `metadata_truncated` when it shortens a title or identity.
If GeoTrace is preparing History, retry the listing.

Pass a History recording ID as `recording` to `open_history_recording`.
GeoTrace reports the operation's current state, including failure to start a History read.
The companion polls within a one-second budget that starts with the request.
A fast open can return `ready` or `awaiting_track_settings` in the first reply.
If the budget expires or a status request fails, the companion returns the latest state it received for that operation.
Poll `get_history_open_status` with the returned `operation` while the read or load is in progress.
Use the `recording` reference in a ready result with `get_recording` and other loaded session tools.
If GeoTrace cannot reproduce the stored track layout, the operation reports `history_track_layout_unavailable`.
For a recording with stored tracks, the operation expires if local processing changes the loaded track layout.
Unloading a track from that recording also expires the operation.
If the status is `awaiting_track_settings`, make the choice in the GeoTrace desktop dialog.
Recalculation through this dialog requires writable History with recording storage enabled.
GeoTrace accepts one active History open and returns the same operation for repeat requests for that recording while it is active.
A new accepted open replaces the previous operation status.
GeoTrace checks the stored recording again before reusing a loaded result.
Open older History recordings in the desktop History window when GeoTrace cannot verify their identity.
When the database is writable and the stored tracks still match, GeoTrace applies saved track visibility before reporting `ready`.
GeoTrace restores saved logs and snap runs in the background.
If several loaded copies share the History entry, unload them and open the entry again to restore its saved logs.
Use the History window to restore saved logs, display settings, and snap runs from a read-only database.

## Inspect session bridge traffic

Open **File → Session bridge traffic…** in GeoTrace with the session bridge enabled.
Select **Capture traffic** to record complete request and reply payloads, including error replies, for this session.
Capture starts off.
Closing the traffic window leaves capture enabled.
GeoTrace keeps a bounded buffer in memory and drops the oldest entries when it fills.
Turn capture off to stop recording and keep the retained entries.
Use **Clear** to remove all captured entries from memory.
Select an entry to see its exact payload and use **Copy payload** to copy it.
GeoTrace displays and copies payloads with invalid UTF-8 or NUL bytes in lossless hex.

## Investigate a recording with MCP

These examples use the repository's [demo recording](../tests/fixtures/demo_trip/demo_trip.gtd).
The [investigation script](examples/investigate.py) uses the returned recording, track and query references throughout each investigation.
Run the commands from the repository root with Python 3.10 or later and both GeoTrace binaries on `PATH`.
Use `--companion` to specify another path to the companion.
Enable **Query highlights** in GeoTrace's map visibility controls before running the script.

### Show an unusual interval on the map and plot

Start GeoTrace with the demo recording, then run the Python command from a second terminal.

```sh
geotrace --offline --session-bridge tests/fixtures/demo_trip/demo_trip.gtd
```

```sh
python3 docs/examples/investigate.py loaded
```

The demo has one track with 151 points and 151 satellite reports.
The script checks and runs `points | where eph > 20 m | table eph, sats_seen, sats_fix`.
It reads the match and its rows, inspects two surrounding points on either side, and frames the match on the map and plot.
GeoTrace enables **EPH (m)**, **Sats seen**, and **Sats fix** in the plot.

The query returns one match at point indices 139 and 140, from 2026-02-01 14:02:19 UTC to 14:02:20 UTC.
Reported EPH is 28.1m and 21.3m, then falls to 16.5m at the next point.
EPH has status `missing` for both points just before the match.

### Investigate low accuracy on a date

Keep recording storage enabled during the first example.
GeoTrace saves the imported demo recording in History.
Quit GeoTrace after the first example and start it again with the same data directory and no recording argument.

```sh
geotrace --offline --session-bridge
```

```sh
python3 docs/examples/investigate.py history
```

The script searches History for 2026-02-01 UTC and opens the matching stored recording.
It waits for `ready` and reads the new loaded recording reference.
The query compares EPH with satellites seen, satellites used, and `util_all`.
For `util_all`, GeoTrace measures the fraction in the fix among satellites at or above the current elevation mask.
The script also opens **Settings > Analysis > Elevation mask**.

The two high-EPH points occur at the start of fix recovery.
The receiver reports 14 satellites in view at both points and uses four, then five.
EPH is the receiver's estimate of horizontal position error.
These readings show that high estimated error coincides with low satellite use during recovery.
Establishing the cause requires further evidence.

To request a temporary elevation mask change, restart GeoTrace without a recording argument and run the History example with a mask of 45°.

```sh
python3 docs/examples/investigate.py history --elevation-mask 45
```

The script compares the same point times and restores the previous setting.
With the default mask of 15°, the demo gives these values.

| Time on 2026-02-01 UTC | EPH | Seen | Used | Share used at 15° | Share used at 45° |
|---|---:|---:|---:|---:|---:|
| 14:02:19 | 28.1m | 14 | 4 | 28.6% | 50.0% |
| 14:02:20 | 21.3m | 14 | 5 | 35.7% | 66.7% |

A higher mask changes which satellites count toward that share.
The reported EPH and satellite counts stay the same.
Queries use the explicit `with mask ... deg` value, so the script sends a new query with the updated value.
The script uses fixed decimal notation to preserve the effective mask in the query.
The plot uses the current desktop mask and the script restores it before framing the plot.

In a session with several loaded recordings, the query runs over all enabled tracks in the global filter.
The script keeps matches for the selected recording by comparing the returned recording references.
Use a session with one demo recording if the script reports multiple candidates.
For recordings spanning several days, set the desktop's global time filter to the UTC day of interest before running the date investigation.
