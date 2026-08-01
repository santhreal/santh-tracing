//! Invariant: init returns a guard.

use santh_tracing::{init, LogLevel};

#[test]
fn init_returns_guard() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard = init("unit_init_guard", LogLevel::Info);
}
