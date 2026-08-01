//! Contract: `default_filter()` applies when the environment is unset and
//! overrides the bare `level` fallback.

use santh_tracing::{init_with, InitConfig, LogLevel};

#[test]
fn default_filter_overrides_level_when_env_unset() {
    std::env::remove_var("RUST_LOG");
    let path = std::env::temp_dir().join(format!(
        "santh_tracing_deffilter_{}.log",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    // level is Trace (would allow info), but default_filter("warn") must win.
    let cfg = InitConfig::new("deffilter-test", LogLevel::Trace)
        .default_filter("warn")
        .file_sink(&path)
        .expect("temp log file opens for append");
    let _guard = init_with(cfg);

    santh_tracing::tracing::info!("info must be filtered out");
    santh_tracing::tracing::warn!("warn must pass");

    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert!(
        contents.contains("warn must pass"),
        "warn must pass default_filter(\"warn\"); got: {contents:?}"
    );
    assert!(
        !contents.contains("info must be filtered out"),
        "default_filter(\"warn\") must override level=Trace and drop info; got: {contents:?}"
    );
}
