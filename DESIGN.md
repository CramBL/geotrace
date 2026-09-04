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
Write one sentence per line in markdown files (`.md`, docs, READMEs, agent guides).
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
Text a user copies out - an identity, a path, a version, a log line - opts back in with `Label::new(…).selectable(true)`.
In a clickable row, register the row's own `Sense` above its labels with `Ui::interact_opt` and `InteractOptions { move_to_top: true }`, as the log viewer's table does: a selectable label senses clicks and drags too.
A row holding an interactive control, such as the Visible section's track rows with their checkbox, registers with `move_to_top: false` instead, which leaves the row's sense under the control and lets the control keep its own clicks.
The row then keeps its hover text and its click, the labels keep the drag that selects, and egui reports no click when the pointer travelled.
Offer a copy button (as the query results do) where such a value is worth copying.
