//! Invariant: calling init twice does not panic.

use santh_tracing::{init, LogLevel};

#[test]
fn init_twice() {
    let _lock = crate::support::INIT_LOCK.lock();
    let _guard1 = init("init_twice_1", LogLevel::Info);
    let _guard2 = init("init_twice_2", LogLevel::Info);
}
