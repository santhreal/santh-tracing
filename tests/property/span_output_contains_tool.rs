//! Invariant: span output always contains the expected tool= prefix.

use proptest::prelude::*;

proptest! {
    #[test]
    fn span_output_contains_tool(name in "[a-zA-Z0-9_-]{1,64}") {
        let output = crate::support::capture_output(|| {
            santh_tracing::santh_span!(&name, "op", "target", {
                santh_tracing::tracing::info!("event");
            });
        });
        let expected = format!("tool=\"{name}\"");
        prop_assert!(
            output.contains(&expected),
            "Fix: expected {expected} in output: {output}"
        );
    }
}
