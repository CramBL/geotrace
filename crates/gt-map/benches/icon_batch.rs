//! Throughput of the CPU icon-mesh backend: how long it takes to transform
//! and collect N icon instances into the frame's mesh.
//!
//! The CPU path serves small flush segments and contexts without the
//! GPU-instanced pipeline (`icon_mesh/gpu.rs`), which is the default above
//! `GPU_MIN_INSTANCES`. These numbers guard the CPU path against
//! throughput regressions.

// Benches, like examples, favour brevity: the core's robustness restriction
// lints (no unwrap/expect/panic/indexing) are not enforced on
// measurement-only code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::allow_attributes,
    reason = "benchmark: development-only code"
)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use egui::{Color32, Pos2, Vec2};
use gt_map::icon_mesh::{IconId, IconInstance, IconMeshBatch, IconMeshLibrary};
use std::hint;

/// A realistic mixed workload: mostly nav arrows (fill + outline per fix,
/// rotated, like a dense track), some ghost chevrons, a few markers.
fn instances(count: usize) -> Vec<IconInstance> {
    (0..count)
        .map(|i| {
            let angle = (i as f32) * 0.37;
            let direction = Some(Vec2::new(angle.cos(), angle.sin()));
            // Scatter across a 1920x1080 viewport: x wraps per row, y steps
            // 7 px per row so rows do not overlap exactly.
            let center = Pos2::new((i % 1920) as f32, (((i / 1920) * 7) % 1080) as f32);
            match i % 8 {
                0 => IconInstance {
                    icon: IconId::GhostFix,
                    center,
                    half_extents: Vec2::splat(9.0),
                    direction,
                    tints: [Color32::from_rgb(219, 68, 55); 2],
                },
                1 => IconInstance {
                    icon: IconId::Warning,
                    center,
                    half_extents: Vec2::splat(12.0),
                    direction: None,
                    tints: [Color32::WHITE; 2],
                },
                _ => IconInstance {
                    icon: IconId::NavArrow,
                    center,
                    half_extents: Vec2::splat(9.0),
                    direction,
                    tints: [Color32::from_rgb(66, 133, 244), Color32::WHITE],
                },
            }
        })
        .collect()
}

fn bench_batch_collect(c: &mut Criterion) {
    let library = match IconMeshLibrary::embedded() {
        Ok(library) => library,
        Err(err) => panic!("embedded icon meshes must decode in benches: {err:#}"),
    };
    let mut group = c.benchmark_group("icon_batch_collect");
    for count in [1_000_usize, 10_000, 50_000] {
        let workload = instances(count);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &workload, |b, w| {
            b.iter(|| {
                let mut batch = IconMeshBatch::new(Some(&library), 2.0);
                for instance in w {
                    batch.push(*instance);
                }
                hint::black_box(batch);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_batch_collect);
criterion_main!(benches);
