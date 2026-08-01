//! Contract: `json_output()` emits one JSON object per event carrying the
//! level, message, and structured fields.

use santh_tracing::{init_with, InitConfig, LogLevel};

#[test]
fn json_output_writes_parseable_ndjson() {
    std::env::set_var("RUST_LOG", "info");
    let path = std::env::temp_dir().join(format!("santh_tracing_json_{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let cfg = InitConfig::new("json-test", LogLevel::Info)
        .json_output()
        .file_sink(&path)
        .expect("temp log file opens for append");
    let _guard = init_with(cfg);

    santh_tracing::tracing::info!(request_id = 42, "json event");

    let contents = std::fs::read_to_string(&path).expect("log file readable");
    let _ = std::fs::remove_file(&path);
    let line = contents.lines().next().unwrap_or_default();
    assert!(
        line.trim_start().starts_with('{') && line.trim_end().ends_with('}'),
        "json mode must emit a JSON object per line; got: {line:?}"
    );
    assert!(
        line.contains("\"level\":\"INFO\""),
        "json line must carry the level; got: {line}"
    );
    assert!(
        line.contains("json event"),
        "json line must carry the message; got: {line}"
    );
    assert!(
        line.contains("request_id"),
        "json line must carry the structured field; got: {line}"
    );
}
