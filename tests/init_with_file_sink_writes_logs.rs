//! Contract: `init_with` + `file_sink` diverts logs to the file and honours
//! `RUST_LOG`. One test per binary so the process-global `OnceLock` is fresh.

use santh_tracing::{init_with, InitConfig, LogLevel};

#[test]
fn file_sink_writes_logs_to_file() {
    std::env::set_var("RUST_LOG", "info");
    let path =
        std::env::temp_dir().join(format!("santh_tracing_filesink_{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let cfg = InitConfig::new("filesink-test", LogLevel::Info)
        .file_sink(&path)
        .expect("temp log file opens for append");
    let _guard = init_with(cfg);

    santh_tracing::tracing::info!("hello from the file sink");

    let contents = std::fs::read_to_string(&path).expect("log file readable");
    let _ = std::fs::remove_file(&path);
    assert!(
        contents.contains("hello from the file sink"),
        "file_sink must capture the log line; got: {contents:?}"
    );
}
