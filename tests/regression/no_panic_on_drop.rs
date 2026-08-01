//! Invariant: dropping TracingGuard does not panic.

use santh_tracing::{init, LogLevel};

#[test]
// InitGuard intentionally implements no `Drop` (dropping it must NOT uninstall
// the global subscriber); this explicit `drop` asserts that contract is a
// harmless no-op, so the `drop_non_drop` lint is expected and allowed here.
#[allow(clippy::drop_non_drop)]
fn no_panic_on_drop() {
    let guard = init("drop_tool", LogLevel::Info);
    drop(guard);
}
