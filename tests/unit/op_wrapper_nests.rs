//! Invariant: with_op wraps closures and nesting works correctly.

use santh_tracing::{init, with_op, LogLevel};

#[test]
fn op_wrapper_nests() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard = init("unit_nest", LogLevel::Info);

    let output = crate::support::capture_output(|| {
        with_op("outer", || {
            santh_tracing::tracing::info!("outer event");
            with_op("inner", || {
                santh_tracing::tracing::info!("inner event");
            });
        });
    });

    let outer_line = output
        .lines()
        .find(|l| l.contains("outer event"))
        .expect("Fix: missing outer event");
    let inner_line = output
        .lines()
        .find(|l| l.contains("inner event"))
        .expect("Fix: missing inner event");

    assert!(outer_line.contains("op=\"outer\""), "Fix: outer op missing");
    assert!(
        !outer_line.contains("op=\"inner\""),
        "Fix: outer event should not contain inner op"
    );

    assert!(
        inner_line.contains("op=\"outer\""),
        "Fix: inner event should inherit outer op"
    );
    assert!(inner_line.contains("op=\"inner\""), "Fix: inner op missing");
}
