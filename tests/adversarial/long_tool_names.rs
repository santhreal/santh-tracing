//! Invariant: very long tool names do not panic or truncate.

use santh_tracing::{init, with_op, LogLevel};

#[test]
fn long_tool_names() {
    let _lock = crate::support::INIT_LOCK.lock();
    let long_name = "a".repeat(10_000);
    let long_name: &'static str = Box::leak(long_name.into_boxed_str());
    let _guard = init(long_name, LogLevel::Info);

    let output = crate::support::capture_output(|| {
        with_op("op", || {
            santh_tracing::tracing::info!("event");
        });
    });

    assert!(
        output.contains(long_name),
        "Fix: long tool name should appear in output"
    );
}
