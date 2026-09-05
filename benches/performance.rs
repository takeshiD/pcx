mod support;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use pcx_cli::{pcd, ros2::pointcloud2};
use std::{hint::black_box, path::PathBuf, sync::Arc, time::Duration};
use support::{CountingWriter, EXPECTED_RETAINED_POINTS, Fixture, POINT_COUNT};

fn benchmarks(criterion: &mut Criterion) {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.cropped.dimensions().point_count(),
        EXPECTED_RETAINED_POINTS
    );
    if let Some(path) = std::env::var_os("PCX_BENCH_REPORT") {
        fixture
            .write_report(&PathBuf::from(path))
            .expect("benchmark metrics report must be writable");
    }

    let mut probe = criterion.benchmark_group("probe");
    probe.throughput(Throughput::Bytes(fixture.report.fixture.mcap_size_bytes));
    probe.bench_function("synthetic_mcap", |bencher| {
        bencher.iter(|| black_box(support::probe(black_box(&fixture.mcap_path))));
    });
    probe.finish();

    let mut decode = criterion.benchmark_group("decode");
    decode.throughput(Throughput::Elements(POINT_COUNT as u64));
    decode.bench_function("pointcloud2_cdr", |bencher| {
        bencher.iter(|| {
            black_box(
                pointcloud2::decode(black_box(Arc::clone(&fixture.cdr)))
                    .expect("benchmark CDR must decode"),
            )
        });
    });
    decode.finish();

    let mut operator = criterion.benchmark_group("operator");
    operator.throughput(Throughput::Elements(POINT_COUNT as u64));
    operator.bench_function("axis_aligned_crop", |bencher| {
        bencher.iter(|| {
            black_box(
                fixture
                    .crop
                    .execute_view(black_box(&fixture.view))
                    .expect("benchmark crop must execute"),
            )
        });
    });
    operator.finish();

    let mut encode = criterion.benchmark_group("encode");
    encode.throughput(Throughput::Elements(
        fixture.cropped.dimensions().point_count() as u64,
    ));
    for encoding in [pcd::Encoding::Binary, pcd::Encoding::Ascii] {
        let name = match encoding {
            pcd::Encoding::Binary => "pcd_binary",
            pcd::Encoding::Ascii => "pcd_ascii",
        };
        encode.bench_function(name, |bencher| {
            bencher.iter(|| {
                let mut output = CountingWriter::default();
                pcd::write(&mut output, black_box(&fixture.cropped), encoding)
                    .expect("benchmark PCD must encode");
                black_box(output.bytes)
            });
        });
    }
    encode.finish();
}

criterion_group! {
    name = performance;
    config = Criterion::default()
        .without_plots()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(30);
    targets = benchmarks
}
criterion_main!(performance);
