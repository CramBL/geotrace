//! Throughput of the log line index: how long a journald-shaped log of a given
//! size takes to detect, index and sort into entries.
//!
//! The design budget is an 80 MiB journal indexed in well under a second on a
//! desktop, with 1 GiB still usable. The sizes below bracket both. The
//! sequential path is measured by staying under one chunk: parsing runs on
//! gt-logfile's own pool, so a bench cannot pick the worker count.

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
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use gt_test_utils::log_fixtures::{self, SyntheticLogSpec, SyntheticLogTimestamps};

const MIB: usize = 1024 * 1024;

/// This much text indexes on one thread, under gt-logfile's chunk size.
const SINGLE_CHUNK_BYTES: usize = 12 * MIB;

/// A moment after every generated timestamp, so year inference is stable.
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000, 0).expect("a valid moment")
}

fn fixture(approx_bytes: usize) -> Arc<str> {
    Arc::from(log_fixtures::synthetic_journald_log(SyntheticLogSpec {
        approx_bytes,
        seed: 1,
        timestamps: SyntheticLogTimestamps::SyslogShort,
    }))
}

fn bench_parse_log(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_log");
    group.sample_size(10);

    for approx_bytes in [100 * MIB, 1024 * MIB] {
        let text = fixture(approx_bytes);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{} MiB", approx_bytes / MIB)),
            &text,
            |b, text| {
                b.iter(|| hint::black_box(gt_logfile::parse_log(Arc::clone(text), now())));
            },
        );
    }

    group.finish();
}

/// The sequential path every log below one chunk takes.
fn bench_parse_log_single_chunk(c: &mut Criterion) {
    let text = fixture(SINGLE_CHUNK_BYTES);

    let mut group = c.benchmark_group("parse_log_single_chunk");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("{} MiB", SINGLE_CHUNK_BYTES / MIB)),
        &text,
        |b, text| {
            b.iter(|| hint::black_box(gt_logfile::parse_log(Arc::clone(text), now())));
        },
    );
    group.finish();
}

criterion_group!(benches, bench_parse_log, bench_parse_log_single_chunk);
criterion_main!(benches);
