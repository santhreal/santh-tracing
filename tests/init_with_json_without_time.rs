//! Contract: `without_time()` drops the timestamp field from JSON output.

use santh_tracing::{init_with, InitConfig, LogLevel};

#[test]
fn json_without_time_omits_timestamp_field() {
    std::env::set_var("RUST_LOG", "info");
    let path =
        std::env::temp_dir().join(format!("santh_tracing_notime_{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let cfg = InitConfig::new("notime-test", LogLevel::Info)
        .json_output()
        .without_time()
        .file_sink(&path)
        .expect("temp log file opens for append");
    let _guard = init_with(cfg);

    santh_tracing::tracing::info!("no timestamp here");

    let contents = std::fs::read_to_string(&path).expect("log file readable");
    let _ = std::fs::remove_file(&path);
    let line = contents.lines().next().unwrap_or_default();
    assert!(
        line.contains("no timestamp here"),
        "message must be present; got: {line}"
    );
    assert!(
        !line.contains("\"timestamp\""),
        "without_time() must omit the timestamp field from JSON; got: {line}"
    );
}
