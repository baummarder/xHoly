use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;
use xholy::config::{Config, VideoConfig};
use xholy::core::memory::{regions, Memory};
use xholy::core::EmulationCore;
use xholy::gpu::GpuContext;

fn bench_cpu_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("cpu");
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("frame_step_4096_insns", |b| {
        let mut core = EmulationCore::load(std::path::Path::new("benchmarks/dummy.bin"));
        let mut gpu = GpuContext::new(&Config::default());
        b.iter(|| {
            let _ = core.step_frame(black_box(&mut gpu));
        });
    });

    group.finish();
}

fn bench_gpu_software(c: &mut Criterion) {
    let mut group = c.benchmark_group("gpu");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("software_rasterizer_1080p", |b| {
        let config = Config {
            video: VideoConfig {
                backend: "software".into(),
                width: 1920,
                height: 1080,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut gpu = GpuContext::new(&config);
        let mem = Memory::new();
        b.iter(|| gpu.submit_frame(black_box(&mem)));
    });

    group.finish();
}

fn bench_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory");
    group.bench_function("read_write_4k", |b| {
        let mut mem = Memory::new();
        let base = regions::TITLE;
        b.iter(|| {
            for i in 0..1024u64 {
                mem.write(black_box(base + i * 4), black_box(i), 4);
            }
            let mut acc = 0u64;
            for i in 0..1024u64 {
                acc ^= mem.read(black_box(base + i * 4), 4).unwrap_or(0);
            }
            black_box(acc)
        });
    });
    group.finish();
}

criterion_group!(benches, bench_cpu_step, bench_gpu_software, bench_memory);
criterion_main!(benches);
