//! Invariant: span fields render correctly in output.

use santh_tracing::santh_span;

#[test]
fn span_fields_render() {
    let output = crate::support::capture_output(|| {
        santh_span!("mytool", "myop", "mytarget", {
            santh_tracing::tracing::info!("hello");
        });
    });

    assert!(
        output.contains("tool=\"mytool\""),
        "Fix: tool field missing: {output}"
    );
    assert!(
        output.contains("op=\"myop\""),
        "Fix: op field missing: {output}"
    );
    assert!(
        output.contains("target=\"mytarget\""),
        "Fix: target field missing: {output}"
    );
}
