//! Invariant: metrics API is available and does not panic.

use santh_tracing::{init, metrics, LogLevel};

#[test]
fn metrics_namespace() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard = init("metrics_namespace", LogLevel::Info);

    metrics::counter("requests", &[("status", "200")]);
    metrics::histogram("latency", 0.5, &[]);
    metrics::gauge("connections", 3.0, &[]);
}
#[test]
fn invalid_label_values_or_mismatched_counts_do_not_panic() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard = init("metrics_invalid_labels", LogLevel::Info);

    // Recording metrics with mismatched or invalid labels must emit a warning
    // rather than panicking the process.
    metrics::counter("requests_invalid", &[("status", "200")]);
    metrics::counter("requests_invalid", &[("status", "200"), ("extra", "val")]);
    metrics::histogram("latency_invalid", 0.5, &[("op", "test")]);
    metrics::histogram("latency_invalid", 0.5, &[]);
    metrics::gauge("connections_invalid", 1.0, &[("env", "prod")]);
    metrics::gauge("connections_invalid", 2.0, &[("env", "prod"), ("region", "us")]);
}
