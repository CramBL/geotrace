# Aircraft interference layer

A map overlay and a plot metric showing where aircraft reported degraded GNSS navigation, so a recording can be read against the interference environment it was made in.

## What the data is

gpsjam.org publishes one CSV per UTC day covering the whole world:

```
GET https://gpsjam.org/data/{YYYY-MM-DD}-h3_4.csv
hex,count_good_aircraft,count_bad_aircraft
84005c7ffffffff,412,3
```

Each row counts aircraft inside one H3 resolution-4 cell (about 22 km across) over that day that reported good versus low navigation integrity (ADS-B NIC), as aggregated by adsbexchange.com from volunteer receivers.

Three properties shape every decision below:

1. It measures **aircraft at altitude**, not receivers on the ground.
2. It is a **24-hour aggregate over a 22 km cell**, so it resolves neither the minute nor the kilometre a track needs.
3. **Low aircraft counts make the share meaningless.** Two aircraft, one bad, reads as 50 %.

The feature is therefore named for what it measures - aircraft interference - and never asserts that a track was jammed.

### Host behaviour, probed

- Served gzip-encoded **whether or not the client advertises support**. A transport that does not decode gets compressed bytes that still look like a body.
- Coverage starts **2022-02-14**. Every day from 2022-01-31 through 2022-02-13 answers 404.
- Publication runs one to three days behind, unannounced, so requests are never gated on the lag - the host's answer decides.
- A missing day is a 404 with a JSON body, never an empty CSV. It means "no data published", never "no interference".

## Crates

| Crate | Holds |
| --- | --- |
| `gt-jam` | The domain: calendar, CSV parsing, `JamDataset` index, transport, day selection, and the shared UI wording |
| `gt-jam-store` | The HDF5 archive of published days |
| `gt-store` | The facade over the archive and the recording history |
| `gt-map` | The overlay renderer and the display-toggle row |
| `gt-plot` | The `MetricKind::Jamming` line |
| `src/app/jamming.rs` | The fetch worker and the app's day selection |

`h3o` is the only new third-party dependency: parsing cell indices, mapping a fix to its cell, and getting a cell's boundary ring.

## The archive

Days accumulate in `<data_dir>/geotrace/jamming.h5`, never re-fetched once stored.
The layout is columnar:

```
/observations/{day,cell,good,bad}          extensible, chunked, shuffle+deflate
/days/{day,offset,count,fetched_at,host}   one row per ingested day
```

A day's rows are contiguous, so `/days` turns a date into one slice.
Columns rather than interleaved rows because it compresses far better: one day is 53 KiB as shuffled columns against 161 KiB interleaved, from 891 KiB raw.
A stored day costs about 81 KiB of file.

Zstd was the first choice - it is 8 % smaller - but HDF5's zstd filter is an external plugin absent from this build, and `blosc-sys` links a system library rather than vendoring one.
The schema does not depend on the filter, so it can be swapped without migrating data.

`insert_day` writes the observations before the index entry, so an interrupted insert leaves rows no day points at.
`open_or_create` cuts them.

A day is stored once regardless of which host served it: the dataset is the same either way, and `/days` records the host for provenance rather than for keying.

## What triggers a fetch

Loading a track queues the UTC days its recording spans, capped at `MAX_DAYS_PER_TRACK`.
Days outside the coverage window never reach the network, and days already archived are skipped, so the queue empties for good as the archive fills.
Nothing is fetched at startup, and the overlay does not have to be visible.

Bulk backfilling the coverage window is a separate, explicit action.

## Drawing

Cells draw as filled convex polygons **beneath every track renderer**, on a continuous green-yellow-red ramp anchored at gpsjam's own 2 % and 10 % breakpoints.
Cells under `MIN_AIRCRAFT_FOR_SOLID_FILL` aircraft draw hatched - visible, distinguishable, never dropped.

Viewport culling goes through `JamDataset::observations_within`, which pads the window by one cell radius so a cell reaching into view is kept.
The longitude pad widens toward the poles, and a window crossing the antimeridian widens to every longitude rather than selecting an empty sliver.

Resolution-2 aggregates for low zoom were specified and **dropped**: summing 49 children is aircraft-weighted, and on the captured day it paints 55 of the 108 regions holding an above-breakpoint cell as though they were below it.
If world zoom ever costs measurable frame time, colour a coarse layer by its worst qualifying child instead.

## The plot line

`MetricKind::Jamming` carries one value per fix, in percent, resolved from that fix's own UTC day - so a midnight-crossing track reads two archived days.
The line breaks where no value exists rather than interpolating, the same treatment an unsnapped point gets.

The metric also reaches the query language: `where jamming > 10` selects the stretches under reported interference.

## Saying what the data is

Every surface showing a value names its source, and the wording lives once in `gt_jam::text` so the map, the plot, the toggle and the query docs cannot drift.
A snapshot test pins it.

- The layer is called **Aircraft interference**, never "jamming".
- The value line says "aircraft" - "3 of 415 aircraft reported low navigation accuracy" - so the number cannot be read without seeing what produced it.
- The display toggle's hover carries the airborne-data and resolution caveats.
- Cells below the low-sample threshold say so in words, not only by hatching.
- Six distinct empty states, so an empty map is never mistaken for a clear one: no day chosen, before coverage, in the future, awaiting publication, not downloaded, and published-nothing.

## Privacy

A request carries a date and nothing else, and the response is the whole world.
No recorded position ever leaves the machine, so unlike snap-to-road there is no consent dialog.
The overlay honours `GEOTRACE_OFFLINE`, enforced in `HttpTransport::new` so no code path can reach the network around it.

The base URL is configurable in the settings window for a self-hosted mirror or an offline copy.
