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
