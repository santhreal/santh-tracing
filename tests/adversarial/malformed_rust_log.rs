//! Robustness: `init` must survive a malformed `RUST_LOG` without panicking.
//!
//! This is an end-to-end smoke test only. It deliberately does NOT assert the
//! resulting filter level, because `init` is idempotent through a process-global
//! `OnceLock`: once any sibling adversarial test in this binary has installed the
//! subscriber, this `init` is a silent no-op and `config.install()` never runs.
//! The actual fallback contract (malformed env => the provided level; valid env
//! wins; default_filter / deny_by_default precedence) is asserted directly and
//! in isolation by the `build_filter_tests` unit tests in `src/lib.rs`, which
//! call `build_filter` without touching the global subscriber.

use santh_tracing::{init, InitGuard, LogLevel};

#[test]
fn init_survives_malformed_rust_log() {
    let _lock = crate::support::INIT_LOCK.lock();
    std::env::set_var("RUST_LOG", "invalid=notalevel");
    // Must not panic on a hostile RUST_LOG, and must return a usable guard.
    let guard = init("malformed_rust_log", LogLevel::Warn);
    std::env::remove_var("RUST_LOG");
    // Emitting through whatever subscriber resolved must also not panic.
    tracing::warn!("malformed-rust-log robustness probe");
    // `InitGuard` has no `Drop`; binding it to `_guard`-style keeps it alive
    // to here without pretending it uninstalls anything.
    let InitGuard = guard;
}
