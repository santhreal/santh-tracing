//! Gap (now closed): the default file sink used to forward bytes verbatim, so
//! a tool that configured `file_sink` without wrapping the writer wrote
//! secrets to its log in the clear. The configured sink now redacts
//! unconditionally through `RedactingWriter`, so no tool can leak a secret to
//! its own logs by forgetting to wrap the writer.
//!
//! This test encoded the desired behavior while it was `#[ignore]`d; the
//! default-sink redaction landed in 0.2.1 and the test now runs green.

use santh_tracing::{InitConfig, LogLevel};

#[test]
fn default_file_sink_redacts_secrets() {
    // Per-process temp dir so concurrent test runs (different processes) never
    // collide on the same path.
    let dir = std::env::temp_dir().join(format!(
        "santh_tracing_gap_default_redaction_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let log = dir.join("gap.log");
    let _ = std::fs::remove_file(&log);

    let contents = {
        let _guard = InitConfig::new("gap_tool", LogLevel::Info)
            .file_sink(&log)
            .expect("open log file")
            .init();

        santh_tracing::tracing::info!(
            "leaked token=ghp_0123456789abcdefghijklmnopqrstuvwxyzABCD"
        );

        std::fs::read_to_string(&log).unwrap_or_default()
    };

    // Clean up before asserting so a failure does not leave the temp dir behind.
    let _ = std::fs::remove_dir_all(&dir);

    // REQUIRED: the secret never reaches the sink in the clear.
    assert!(
        !contents.contains("ghp_0123456789"),
        "secret leaked to default sink: {contents}"
    );
    assert!(
        contents.contains("[REDACTED]"),
        "expected redaction marker in sink output: {contents}"
    );
    assert!(
        contents.contains("leaked"),
        "non-secret content must survive redaction: {contents}"
    );
}
