#[path = "../benches/support/mod.rs"]
mod support;

use support::{
    ASCII_OUTPUT_BUDGET_BYTES, BINARY_OUTPUT_BUDGET_BYTES, EXPECTED_RETAINED_POINTS, Fixture,
    PEAK_MANAGED_MEMORY_BUDGET_BYTES, POINT_COUNT,
};

#[test]
fn deterministic_benchmark_metrics_stay_within_declared_budgets() {
    let fixture = Fixture::new();
    let report = &fixture.report;

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.fixture.point_count, POINT_COUNT);
    assert_eq!(report.fixture.cdr_size_bytes, fixture.cdr.len() as u64);
    assert_eq!(
        fixture.view.layout().dimensions().point_count(),
        POINT_COUNT
    );
    assert_eq!(fixture.crop.pipeline().stages().len(), 1);
    assert_eq!(
        fixture.cropped.dimensions().point_count(),
        EXPECTED_RETAINED_POINTS
    );
    assert_eq!(
        report.fixture.retained_point_count,
        EXPECTED_RETAINED_POINTS
    );
    assert!(
        report.measurements.binary_pcd_output_bytes <= BINARY_OUTPUT_BUDGET_BYTES,
        "binary PCD output {} grew beyond its deterministic budget {BINARY_OUTPUT_BUDGET_BYTES}",
        report.measurements.binary_pcd_output_bytes,
    );
    assert!(
        report.measurements.ascii_pcd_output_bytes <= ASCII_OUTPUT_BUDGET_BYTES,
        "ASCII PCD output {} grew beyond its deterministic budget {ASCII_OUTPUT_BUDGET_BYTES}",
        report.measurements.ascii_pcd_output_bytes,
    );
    assert!(
        report.measurements.declared_peak_managed_memory_bytes <= PEAK_MANAGED_MEMORY_BUDGET_BYTES,
        "declared managed-memory peak {} grew beyond its deterministic budget {PEAK_MANAGED_MEMORY_BUDGET_BYTES}",
        report.measurements.declared_peak_managed_memory_bytes,
    );
}

#[test]
fn benchmark_probe_reads_the_streamed_fixture() {
    let fixture = Fixture::new();
    let info = support::probe(&fixture.mcap_path);

    assert_eq!(info.message_count, 1);
    assert_eq!(info.channel_count, 1);
    assert_eq!(info.schema_count, 1);
    assert_eq!(info.size_bytes, fixture.report.fixture.mcap_size_bytes);
}
