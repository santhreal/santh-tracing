//! Invariant: unicode op names render correctly.

use santh_tracing::{init, with_op, LogLevel};

#[test]
fn unicode_op_names() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard = init("unicode_tool", LogLevel::Info);

    let output = crate::support::capture_output(|| {
        with_op("😀火箭", || {
            santh_tracing::tracing::info!("event");
        });
    });

    assert!(
        output.contains("😀火箭"),
        "Fix: unicode op name should appear in output"
    );
}
