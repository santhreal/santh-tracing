//! Contract: `deny_by_default()` emits nothing - not even errors - when no
//! filter environment variable is set.

use santh_tracing::{init_with, InitConfig, LogLevel};

#[test]
fn deny_by_default_emits_nothing_without_env() {
    std::env::remove_var("RUST_LOG");
    let path = std::env::temp_dir().join(format!("santh_tracing_deny_{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let cfg = InitConfig::new("deny-test", LogLevel::Info)
        .deny_by_default()
        .file_sink(&path)
        .expect("temp log file opens for append");
    let _guard = init_with(cfg);

    santh_tracing::tracing::error!("must be suppressed");
    santh_tracing::tracing::info!("must also be suppressed");

    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert!(
        contents.is_empty(),
        "deny_by_default with no RUST_LOG must emit nothing; got: {contents:?}"
    );
}
