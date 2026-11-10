# Map policy measurements

The completed policy reduced total preparation and candidate resolution time in the measured workloads.
At 128 tracks, the combined workload means were 17.1% to 46.0% lower than repaired trunk.
The display-presence optimization removed navigation classification when both fix display categories have the same visibility.

These measurements complete the performance evidence for [#924](https://github.com/CramBL/geotrace/issues/924) and [#915](https://github.com/CramBL/geotrace/issues/915).
The preceding cross-highlight change adds current-frame proof consumption and compile-fail coverage for [#923](https://github.com/CramBL/geotrace/issues/923).

## Revisions and environment

| Name | Production revision | Benchmark changes |
| --- | --- | --- |
| Historical | `2790bd81f2438df7b37b6c533339d16c5bfbd37e` | [Historical comparison patch](map-policy-historical.patch) |
| Repaired trunk | `b1a10a3d91e50330fcf25c03b81fad8ac789da09` | This report's benchmark file |
| Before display optimization | `418bf20f4dfe68113046c0d84872b236c2c19d27` | This report's benchmark file |
| Completed policy | `5b2c4847bbb6835be070dab221e3a826fed20743` | This report's benchmark file |

Measurements ran on 11 November 2026 with Linux `7.0.0-42-generic`, an Intel Core Ultra 7 255H, rustc `1.99.1` (`58cf8f9c3`) and Criterion `0.8.2`.
Timed runs used CPU affinity 2 and the existing `powersave` governor.
Each workload used 50 samples, a 2s warm-up and a 3s measurement window.
Compilation and tests completed before each timed run.
The release profile used optimization level 3, LTO and one code generation unit.

Values below use Criterion's `mean.point_estimate` and 95% bootstrap time confidence intervals from `estimates.json`.
[The results CSV](map-policy-results.csv) contains all 16 modern workload means and time intervals in nanoseconds.
Historical source includes different navigation data and locked dependencies, so its results compare complete revisions.

## Combined preparation and resolution

Each iteration prepares 128 tracks with 100 fixes per track and resolves the selected indexed fixes.
The static spatial index is built before timing.
The benchmark asserts the candidate count for each workload.

| Candidates per track | Historical mean | Repaired trunk mean | Completed policy mean | Change from repaired trunk |
| --- | --- | --- | --- | --- |
| 1 | 2.367µs | 2.820µs | 2.337µs | -17.1% |
| 8 | 10.180µs | 10.848µs | 6.400µs | -41.0% |
| 64 | 75.410µs | 80.524µs | 43.449µs | -46.0% |

The historical sparse comparison has overlapping time intervals: 2.341–2.394µs historically and 2.304–2.373µs for the completed policy.
Its mean ratio is -1.3%.
The historical moderate and dense mean ratios are -37.1% and -42.4%, with disjoint time intervals.

Against the immediate pre-optimization revision, combined means decreased by 15.9%, 38.2% and 44.1%.
The mixed candidate batch decreased from 60.58ns to 52.93ns.
Criterion's relative-mean interval for that change was -13.6% to -11.6%, with a -12.6% point estimate.

## Component measurements

| Workload | Historical mean | Repaired trunk mean | Completed policy mean |
| --- | --- | --- | --- |
| Prepare 8 tracks, 100 fixes each | 85.07ns | 136.50ns | 138.12ns |
| Prepare 128 tracks, 100 fixes each | 1.169µs | 1.612µs | 1.551µs |
| Prepare 1,024 tracks, 100 fixes each | 9.456µs | 13.359µs | 13.732µs |
| Prepare 128 tracks, 10,000 fixes each | 1.212µs | 1.554µs | 1.621µs |
| Resolve 7 spatial candidates | 75.81ns | 58.92ns | 52.93ns |
| Nearest candidate, visible first | 46.39ns | 37.99ns | 35.76ns |
| Nearest candidate, 8 hidden first | 170.61ns | 120.83ns | 118.93ns |
| Prepare 128 marker-heavy tracks | Not measured | 2.238µs | 2.253µs |
| Count marker-heavy data, cold | Not measured | 11.360ms | 11.400ms |
| Count marker-heavy data, cached | Not measured | 48.36ns | 47.23ns |
| Fit marker-heavy data | Not measured | 10.658ms | 10.697ms |
| Resolve candidates with 8 query layers | Not measured | 225.49ns | 156.21ns |
| Count data with 8 query layers | Not measured | 58.423µs | 59.442µs |

Historical preparation computed render entries alone.
The completed plan first prepares reusable track eligibility and file offsets, then computes render entries.
At 128 tracks, this increased preparation by about 0.38µs while eliminating repeated track policy work during candidate resolution.
The combined measurements above include both costs.

Preparation with 100 times as many fixes measured 1.621µs against 1.551µs for the same 128 tracks.
Existing guards verify at most one track-filter evaluation per track, inline storage for typical plans and allocation-free prepared resolution.
Ordinary preparation iterates files and tracks.
Requested count and fit scans remain separate operations.

Scalar changes from repaired trunk remain visible in the table and CSV.
Preparation means changed by -3.8% at 128 tracks, +2.8% at 1,024 tracks and +4.3% for the larger point count.
The preparation routine and eligibility-only scan routines are unchanged from repaired trunk.
A repeated source-identical frame-only run changed means by +1.4%, +1.1%, -1.9% and +0.1% in table order.
Criterion classified those repeated changes as unchanged or within its noise threshold.
The benchmark executable was rebuilt before that control, so it combines host variation and possible binary layout variation.

The original historical candidate batch resolved 9 identities, including a satellite report and a missing fix.
Its 82.62ns mean is retained as a separate CSV row.
The comparison patch adds the same 7 spatial candidates as the modern workload through the historical production visibility entry point.
The historical fixture creates its estimated fix through the legacy satellite report.
Modern fixtures set `SolutionState::NoFix` explicitly.

## Reproduction

Run from a checkout containing this report and benchmark file.
Use a separate Cargo target directory for each source checkout.
Sharing local crate artifacts between revisions produced a stale-library build failure during the investigation.
Registry dependency caches can be shared.
The historical and repaired build logs record compilation of their local crates.
The later stale-library failure produced no timing results.
Final measurements used an isolated target directory.

```sh
policy_repo=$PWD
git worktree add --detach ../geotrace-policy-old 2790bd81f2438df7b37b6c533339d16c5bfbd37e
git -C ../geotrace-policy-old apply "$policy_repo/docs/performance/map-policy-historical.patch"
git worktree add --detach ../geotrace-policy-before b1a10a3d91e50330fcf25c03b81fad8ac789da09
git worktree add --detach ../geotrace-policy-after 5b2c4847bbb6835be070dab221e3a826fed20743
cp crates/gt-map/benches/map_policy.rs ../geotrace-policy-before/crates/gt-map/benches/map_policy.rs
cp crates/gt-map/benches/map_policy.rs ../geotrace-policy-after/crates/gt-map/benches/map_policy.rs
```

Build each revision with its own target directory before pinning the timed process.
The following commands show the completed revision.
Run the same commands for the other checkouts with distinct target directories and baseline labels.

```sh
cd ../geotrace-policy-after
export CARGO_TARGET_DIR="$policy_repo/../geotrace-policy-after-target"
just bench-map-policy --test
taskset -c 2 just bench-map-policy --save-baseline completed --sample-size 50 --warm-up-time 2 --measurement-time 3
```

The recorded labels were `historical-pinned-final`, `repaired-pinned` and `final-pinned` for the first three revisions.
The completed revision used `--baseline final-pinned` after rebuilding the same source checkout with the display optimization.
The source-identical control used `frame_policy --baseline optimized-pinned` with the same sampling settings.
Copy saved Criterion JSON directories into matching workload paths when comparing baselines across target directories.
Copy only measurement data across revisions.

An earlier unpinned comparison used `--sample-size 30 --warm-up-time 1 --measurement-time 2` against `before-cross-highlight`.
Its preparation shifts prompted the pinned runs and combined workload.
The pinned measurements in the CSV are the acceptance evidence.
Raw logs, samples, environment details and investigation controls remain in the local excluded `AUDITS/map-policy/` directory.

Run `just ci-essentials` after restoring the completed source tree.
It checks the policy properties, compile-fail boundaries, compiled equivalence, complexity and allocation guards alongside the workspace suite.
