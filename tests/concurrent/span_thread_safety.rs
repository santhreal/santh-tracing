//! Invariant: spans can be created and logged from multiple threads safely.

use santh_tracing::{init, with_op, LogLevel};
use std::thread;

#[test]
fn span_thread_safety() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard = init("concurrent_tool", LogLevel::Info);

    let handles: Vec<_> = (0..10)
        .map(|i| {
            thread::spawn(move || {
                let op = format!("op_{i}");
                // with_op takes `&str` (any lifetime), so a local borrow works;
                // no need to Box::leak a 'static string (which leaked per thread).
                crate::support::capture_output(|| {
                    with_op(&op, || {
                        santh_tracing::tracing::info!("thread {i}");
                    });
                })
            })
        })
        .collect();

    for (i, h) in handles.into_iter().enumerate() {
        let out = h.join().expect("Fix: thread panicked");
        assert!(
            out.contains(&format!("op=\"op_{i}\"")),
            "Fix: missing op for thread {i}"
        );
        assert!(
            out.contains(&format!("thread {i}")),
            "Fix: missing event for thread {i}"
        );
    }
}
