# GeoTrace query language

## Pipelines

Start with `points` for navigation points or `@channel` for a whole channel's samples.
Separate stages with `|`.
Place optional `with` and `window` stages before the `where` conditions, in that order.
Use one display mode and one `table` stage at most, after the conditions and in either order.
Separate point queries with blank lines to run them in order.
Run a channel-source query by itself.

## Channels

Use `@channel` for a scalar channel value and `@channel.component` for a vector component.
Use `norm(@channel)` for the magnitude of a vector.
The channel name and component must be present in the recording channel schema.

With `points`, use channel values inside an aggregate.
A channel aggregate reads samples within the navigation window's time span.
Each aggregate reads navigation points or samples from one channel.
An aggregate can combine components of the same channel.
Compare separate aggregates to combine values from different timelines.
A channel source can read only its own samples and components.
Use `points` to query built-in navigation metrics.
Aggregate the sample values in a channel-source predicate with a window.
Use aggregates for explicit table columns on a channel source.
On the map, channel matches affect the track segments at the matching sample times.

## Expressions and units

Use parentheses to group expressions.
Write powers with an integer exponent, as in `velocity^2` or `velocity²`.
Use equality operators to compare a count with another count or a bare number.
Aggregate calls require a `window`, including calls in `table`.
In a windowed predicate, aggregate navigation metrics and channel values before comparing them.

Write fixed decimal numbers with a compatible unit for measured quantities, as in `30 km/h`.
Write counts and unitless values as bare numbers.
The checker treats missing and unrecognized channel units as unitless.

## Complete examples

Find points with high estimated horizontal error and at least three satellites in view beyond those used in the fix.

```text
points | where eph > 20 m and (sats_seen - sats_fix) >= 3 | draw | table eph, sats_seen, sats_fix
```

Keep points with low satellite utilization at a 15° elevation mask.

```text
points | with mask 15 deg | where util_all < 50 % | keep
```

Find ten-point windows with a small heading spread and a large acceleration component.
This example uses a vector channel `accel` in `g` with components `x`, `y`, and `z`.

```text
points | window 10 | where spread(heading) < 10 deg and max(@accel.x) > 1 g | table spread(heading), max(@accel.x)
```

Match individual samples from a scalar channel `speed` in `km/h`.

```text
@speed | where @speed >= 15 km/h
```

Find two-second windows with a high average vector magnitude on the `accel` sample timeline.
Hide the map segments covered by those matches.

```text
@accel | window 2 s | where avg(norm(@accel)) > 1 g | hide | table max(@accel.x)
```
