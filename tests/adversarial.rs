//! Adversarial tests for santh-tracing.

#[path = "support/mod.rs"]
mod support;

#[path = "adversarial/init_twice.rs"]
mod init_twice;
#[path = "adversarial/long_tool_names.rs"]
mod long_tool_names;
#[path = "adversarial/malformed_rust_log.rs"]
mod malformed_rust_log;
#[path = "adversarial/secret_redaction.rs"]
mod secret_redaction;
#[path = "adversarial/unicode_op_names.rs"]
mod unicode_op_names;
