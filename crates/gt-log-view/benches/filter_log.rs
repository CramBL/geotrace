//! Throughput of the filter engine: how long one generation of a filter's scan
//! takes over a journald-shaped log of a given size.
//!
//! The design budget is a full regex pass over 1 GiB across the cores in a few
//! hundred milliseconds, with plain terms well under that. `live_filter_edit`
//! measures what one keystroke costs on a 100 MiB log: the compile, the scan,
//! and composing the entries the table then shows.

// Benches, like examples, favour brevity: the core's robustness restriction
// lints (no unwrap/expect/panic/indexing) are not enforced on
// measurement-only code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::allow_attributes,
    reason = "benchmark: development-only code"
)]

use std::{hint, sync::Arc};

use chrono::{DateTime, Utc};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use gt_log_view::FilterStack;
use gt_logfile::ParsedLog;
use gt_test_utils::log_fixtures::{self, SyntheticLogSpec, SyntheticLogTimestamps};

const MIB: usize = 1024 * 1024;

/// Three terms of a line a real journal writes often, none of them the first
/// thing on the line.
const PLAIN_TERMS: &str = "navsyncd uploaded queue";

/// A pattern the literal optimizer cannot reduce to one substring search: an
/// alternation of two units, a digit run, and a suffix.
const REGEX: &str = r"(gpsd|navsyncd)\[\d+\]: .*(fix|queue)\b";

/// A moment past every generated timestamp, for stable year inference.
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000, 0).expect("a valid moment")
}

fn fixture(approx_bytes: usize) -> Arc<ParsedLog> {
    let text = log_fixtures::synthetic_journald_log(SyntheticLogSpec {
        approx_bytes,
        seed: 1,
        timestamps: SyntheticLogTimestamps::SyslogShort,
    });
    Arc::new(gt_logfile::parse_log(text.into(), now()).expect("the fixture log parses"))
}

/// One generation of the scan: a fresh stack takes the filter and blocks until
/// its matches land.
fn scan_once(log: &Arc<ParsedLog>, text: &str, regex: bool) {
    let mut stack = FilterStack::new(Arc::clone(log));
    stack.set_live_filter_regex(regex);
    stack.set_live_filter_text(text);
    stack.wait_for_queries();
    hint::black_box(stack.visible_entries().len());
}

fn bench_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_scan");
    group.sample_size(10);

    for approx_bytes in [100 * MIB, 1024 * MIB] {
        let log = fixture(approx_bytes);
        let size = format!("{} MiB", approx_bytes / MIB);
        group.throughput(Throughput::Bytes(log.text().len() as u64));

        group.bench_with_input(BenchmarkId::new("plain terms", &size), &log, |b, log| {
            b.iter(|| scan_once(log, PLAIN_TERMS, false))
        });
        group.bench_with_input(BenchmarkId::new("regex", &size), &log, |b, log| {
            b.iter(|| scan_once(log, REGEX, true))
        });
    }

    group.finish();
}

/// What one keystroke costs while the user types into the live filter, on a log
/// of the size a real device export reaches.
fn bench_live_filter_edit(c: &mut Criterion) {
    let log = fixture(100 * MIB);

    let mut group = c.benchmark_group("live_filter_edit");
    group.sample_size(20);
    group.throughput(Throughput::Bytes(log.text().len() as u64));
    group.bench_function(BenchmarkId::from_parameter("100 MiB"), |b| {
        b.iter_batched(
            || FilterStack::new(Arc::clone(&log)),
            |mut stack| {
                stack.set_live_filter_text(PLAIN_TERMS);
                stack.wait_for_queries();
                hint::black_box(stack.visible_entries().len())
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_scan, bench_live_filter_edit);
criterion_main!(benches);
