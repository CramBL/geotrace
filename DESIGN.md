# Design guidelines
Guidelines for UI design, covering GUI, CLI, documentation, log messages, etc

## Text
#### Sentences
Short, single-sentence text do NOT end in a period in the UI.

Multi-sentence text ALWAYS ends in a period.

#### Casing
We prefer normal casing, and avoid Title Casing.

We only use Title Casing for company and product names (Rerun, Rerun Viewer, Discord, …), but NOT for concepts like “container”, “view”, etc.

#### Examples
Good: `log("File saved")`
Bad: `log("file saved.")`

#### Units
Write a number and its unit without a space: `30%`, `2h`, `5min`, `22km`, `0.16m`, `48.9°N`.
Three kinds of unit keep the space: a token longer than two letters (`48 TECU`, `40 dB-Hz`), a data size (`126.6 MB`), and a unit written with a slash (`5.0 km/h`, `2 m/s`).
A comparison operator takes a space before a signed number (`< -30%`, `> +43%`) and none before an unsigned one (`≥5`, `≥2%`).
The reference documents (`reference.rs` in each crate) keep the space throughout.
A query's text follows the language's own syntax (`velocity > 30 km/h`).

#### Dates and times
Write an instant as `2024-05-10 14:32 UTC`, and to the second (`2024-05-10 14:32:05 UTC`) where it is a fix or a plot sample.
No `T` between the date and the time, and no parentheses around `UTC`.
The two formats are `gt_fmt::UTC_MINUTE_FORMAT` and `gt_fmt::UTC_SECOND_FORMAT`.

#### Dashes
Use a spaced hyphen (` - `) for parenthetical breaks in prose (docs, comments, log messages).
Em dashes (`—`) are reserved for UI display only (e.g. as a placeholder for absent values via `gt_ui_theme::EM_DASH`).

En dashes are reserved for numeric/range expressions (`2020–2025`, `pp. 10–15`, `~3–4 GB`).

#### Line breaks in markdown
Write one sentence per line in markdown files (`.md`, docs, READMEs).
Markdown joins consecutive non-empty lines into a single paragraph, so this does not affect rendering - but it produces much cleaner diffs.
Each edited sentence shows up as a single changed line.

Use a blank line between paragraphs as usual.

### Buttons

When a button action requires more input after pressing, suffix it with `…`.

Good: `Save recording…` (leads to a save-dialog)

### Controls and conditional state

Never hide or remove a control because another setting is off.
Render it disabled (grayed out via `ui.add_enabled(false, …)`) and use hover text to explain why it is inactive and what the user needs to enable first.
This keeps the layout stable and makes the feature discoverable - a hidden control teaches the user nothing.

Good: DragValue grayed out, tooltip reads "Tick 'Auto-prune when over' to set a threshold"

Bad: DragValue hidden when the auto-prune checkbox is unchecked

Every interactive control should have hover text when it is disabled.
Active controls should also have hover text when their purpose is not obvious from the label alone.

## GUI labels

We do not use a colon suffix for labels in front of a value.

Good: `Color 🔴`
Bad: `Color: 🔴`

### Text selection

Labels do not select: `gt_ui_theme::install_app_style` turns egui's default off for the whole app, so no readout, caption or header shows a text I-beam.
Use `Label::new(…).selectable(true)` for copyable labels such as identities, paths and versions.
In a clickable row with selectable labels, register the row's own `Sense` above its labels with `Ui::interact_opt` and `InteractOptions { move_to_top: true }`.
A row holding an interactive control, such as the Visible section's track rows with their checkbox, registers with `move_to_top: false` instead, which leaves the row's sense under the control and lets the control keep its own clicks.
The row then keeps its hover text and its click, the labels keep the drag that selects, and egui reports a click only where the pointer stayed put.
The virtualized log table records its selection in displayed row order across the scrollable area.
Drag with a mouse or on a touchscreen to select log text.
Double-click selects a word, triple-click selects a displayed row, and the platform Select All command selects the displayed document.
Shift with an arrow key extends or shrinks the selection from its active end.
Shift+Up and Shift+Down retain the horizontal position across displayed rows.
Cut copies the selected log text to the clipboard while leaving the log intact.
Offer a copy button (as the query results do) where such a value is worth copying.

## MCP interface

Apply these rules in reviews of new MCP tools and protocol changes.

### Tool scope

Define tools for complete user operations, such as framing a query match on the map.
Use registry keys as parameters for operations over settings and metrics.
Include the total tool count in each tool addition review.
Require a written justification for additions once the total approaches 25–30 tools.

### Desktop state

Use the same application state and actions as the desktop controls.
Present map highlights, plot ranges, query drafts and setting controls in the desktop interface.
Notify the desktop user when an MCP action changes the view.
Record raw bridge traffic only in opt-in debug or audit logs.

### Queued actions

Check references and desktop preconditions before acknowledging a queued action.
Return an error when a precondition prevents the action.
For a map highlight, check that the display category is enabled.

### Payload limits and ordering

Enforce hard item limits per page and byte limits for encoded requests and replies.
Return a continuation cursor when more items are available.
Flag truncated values explicitly.
Use RFC 3339 UTC timestamps ending in `Z` for instants in requests and replies.
Document each result order and test it across page boundaries.

### Asynchronous operations

Wait briefly on the server for asynchronous operations that usually finish quickly.
Keep the wait within the request deadline.
Include completed results in the initial reply when work finishes during the wait.
Return a polling handle if the operation is still running when the wait ends.

### Tool descriptions and reference material

Keep tool descriptions concise and state the operation, inputs and result.
Offer detailed reference material and examples through on-demand help tools or MCP resources.
