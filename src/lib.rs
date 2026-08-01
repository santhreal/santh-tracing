#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::todo,
        clippy::unimplemented,
        clippy::panic
    )
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc
)]
//! Consistent tracing shape for Santh CLI tools.
//!
//! # Quick start
//!
//! ```
//! use santh_tracing::{init, LogLevel};
//!
//! let _guard = init("mytool", LogLevel::Info);
//! santh_tracing::tracing::info!("ready");
//! ```
//!
//! # Safe-defaults answers
//!
//! - Input size: log-line size tracks what the caller emits; the optional
//!   [`RedactingWriter`] buffers one line at a time, never the whole stream.
//! - Recursion depth: operation spans nest only as deep as the caller's
//!   [`with_op`] calls; the library itself adds no recursion.
//! - Outbound network: none. The crate performs no network access.
//! - Process spawning: none. The crate spawns no child processes.
//! - Filesystem writes: none by default (logs go to stderr). A file is opened
//!   for append only when the caller explicitly sets
//!   [`InitConfig::file_sink`], which validates the path up front.
//! - Credential exposure: every sink (stderr and file) runs through
//!   [`RedactingWriter`], which masks secret shapes line-by-line through
//!   `santh-error`'s redactor before bytes reach the destination.

mod error;
pub mod metrics;
mod redacting_writer;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::{
    fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer, Registry,
};

pub use error::Error;
pub use metrics::*;
pub use redacting_writer::RedactingWriter;

/// Log level configured at init time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Trace-level diagnostics.
    Trace,
    /// Debug-level diagnostics.
    Debug,
    /// Info-level diagnostics.
    Info,
    /// Warning-level diagnostics.
    Warn,
    /// Error-level diagnostics.
    Error,
}

/// Guard returned by [`init`]; dropping does not uninstall the global subscriber.
pub struct InitGuard;

static INIT: OnceLock<()> = OnceLock::new();
/// Process-global tool name. Must NOT be thread-local: metrics and spans are
/// emitted from spawned worker threads that never call `init`, and a thread-local
/// would leave those threads with an empty name (mis-namespacing every metric
/// under `santh__`). One process runs as one tool, so a single global is correct.
static TOOL: RwLock<String> = RwLock::new(String::new());
/// Monotonic counter bumped every time [`set_tool_name`] changes the tool name.
/// The Prometheus facade caches resolved metric vectors per-thread keyed on this
/// generation, so a tool-name change invalidates every cached (and now
/// mis-namespaced) entry without the hot path having to read the tool string.
static TOOL_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Serializes tests that mutate the process-global tool name so they never
/// overlap each other (or the metrics-cache tests) and split another test's
/// metric counts across two tool prefixes. Test-only.
#[cfg(test)]
pub(crate) static TOOL_SERIAL: Mutex<()> = Mutex::new(());
thread_local! {
    static OP_STACK: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Set the process-global tool name and bump [`TOOL_GENERATION`]. The single
/// owner of tool-name writes so the generation counter can never drift out of
/// sync with the name (every writer goes through here).
pub(crate) fn set_tool_name(tool: String) {
    *TOOL.write().unwrap_or_else(std::sync::PoisonError::into_inner) = tool;
    TOOL_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Current tool-name generation. Read on the metrics hot path to detect a
/// tool-name change without cloning (or even reading) the tool string.
#[cfg(feature = "prometheus")]
pub(crate) fn tool_generation() -> u64 {
    TOOL_GENERATION.load(Ordering::Relaxed)
}

/// Install the global Santh tracing subscriber for a tool process.
///
/// Equivalent to `InitConfig::new(tool, level).init()`: the filter comes from
/// `RUST_LOG` when set, otherwise `level`. Logs are written to **stderr** so
/// machine-readable findings on stdout are never corrupted (the Santh logging
/// contract). For JSON output, a log file, or a custom filter, use
/// [`init_with`] with an [`InitConfig`].
pub fn init(tool: &str, level: LogLevel) -> InitGuard {
    init_with(InitConfig::new(tool, level))
}

/// Fallible version of [`init`].
///
/// Returns [`Error::SubscriberAlreadySet`] if a subscriber has already been
/// installed in this process. Otherwise installs the subscriber and returns
/// the guard.
pub fn try_init(tool: &str, level: LogLevel) -> Result<InitGuard, Error> {
    try_init_with(InitConfig::new(tool, level))
}

/// Declarative configuration for [`init_with`].
///
/// One entry point, many shapes: every tool configures tracing by chaining
/// builder methods rather than reaching for `tracing_subscriber` directly.
/// The default is the Santh house style - stderr, full human format, target
/// suppressed, `RUST_LOG`-or-`level` filtering.
#[derive(Debug, Clone)]
pub struct InitConfig {
    tool: String,
    level: LogLevel,
    deny_by_default: bool,
    format: OutputFormat,
    without_time: bool,
    file_path: Option<PathBuf>,
    default_filter: Option<String>,
    env_var: Option<String>,
}

/// Human/JSON/compact event format. The three are mutually exclusive, so a
/// single field captures the choice (the last builder call wins) instead of a
/// pair of `bool`s that could both be set.
#[derive(Debug, Clone, Copy)]
enum OutputFormat {
    Human,
    Json,
    Compact,
}

impl InitConfig {
    /// Start a configuration for `tool`, defaulting to `level` when the
    /// environment does not specify a filter.
    #[must_use]
    pub fn new(tool: impl Into<String>, level: LogLevel) -> Self {
        Self {
            tool: tool.into(),
            level,
            deny_by_default: false,
            format: OutputFormat::Human,
            without_time: false,
            file_path: None,
            default_filter: None,
            env_var: None,
        }
    }

    /// Emit nothing when the filter environment variable is unset, instead of
    /// falling back to `level`. For tools that must stay silent unless the
    /// operator opts in (e.g. via `RUST_LOG`).
    #[must_use]
    pub fn deny_by_default(mut self) -> Self {
        self.deny_by_default = true;
        self
    }

    /// Emit newline-delimited JSON (one object per event) instead of the
    /// human format. Mutually exclusive with [`compact`](Self::compact); the
    /// last one set wins.
    #[must_use]
    pub fn json_output(mut self) -> Self {
        self.format = OutputFormat::Json;
        self
    }

    /// Use the compact single-line human format. Mutually exclusive with
    /// [`json_output`](Self::json_output); the last one set wins.
    #[must_use]
    pub fn compact(mut self) -> Self {
        self.format = OutputFormat::Compact;
        self
    }

    /// Suppress the timestamp field. Useful for deterministic test output and
    /// for tools whose host (journald, the TUI) already timestamps lines.
    #[must_use]
    pub fn without_time(mut self) -> Self {
        self.without_time = true;
        self
    }

    /// Set a default filter directive string (e.g. `"info,chromiumoxide=error"`)
    /// used when the filter environment variable is unset. Takes precedence
    /// over the bare `level` fallback.
    #[must_use]
    pub fn default_filter(mut self, directives: impl Into<String>) -> Self {
        self.default_filter = Some(directives.into());
        self
    }

    /// Read the filter from a tool-specific environment variable name (e.g.
    /// `"WARPSCAN_LOG"`) instead of the default `RUST_LOG`.
    #[must_use]
    pub fn env_var(mut self, name: impl Into<String>) -> Self {
        self.env_var = Some(name.into());
        self
    }

    /// Write logs to `path` (opened for append, ANSI colour off) instead of
    /// stderr. Intended for TUI tools whose terminal is owned by the UI.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if `path` cannot be opened for append, so the
    /// caller can fall back loudly rather than silently losing logs.
    pub fn file_sink(mut self, path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        // Validate up front so the caller learns of the failure here, not at
        // the first dropped log line.
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        self.file_path = Some(path);
        Ok(self)
    }

    /// Install this configuration as the global subscriber, returning the
    /// process-lifetime [`InitGuard`]. Idempotent: only the first call in a
    /// process installs a subscriber.
    pub fn init(self) -> InitGuard {
        init_with(self)
    }

    /// Resolve the [`EnvFilter`]: the configured environment variable (or
    /// `RUST_LOG`) wins; otherwise `default_filter`, else silence when
    /// `deny_by_default`, else the bare `level`.
    fn build_filter(&self) -> EnvFilter {
        let from_env = match &self.env_var {
            Some(name) => EnvFilter::try_from_env(name),
            None => EnvFilter::try_from_default_env(),
        };
        from_env.unwrap_or_else(|_| {
            if let Some(directives) = &self.default_filter {
                EnvFilter::new(directives.clone())
            } else if self.deny_by_default {
                // "off" disables every level; an empty filter would still
                // admit ERROR (EnvFilter's default directive).
                EnvFilter::new("off")
            } else {
                level_filter(self.level)
            }
        })
    }

    /// Build the boxed fmt layer for the selected format, time, and writer.
    fn fmt_layer(
        &self,
        writer: BoxMakeWriter,
        ansi: bool,
    ) -> Box<dyn Layer<Registry> + Send + Sync> {
        let base = fmt::layer()
            .with_target(false)
            .with_ansi(ansi)
            .with_writer(writer);
        match self.format {
            OutputFormat::Json => {
                let layer = base.json();
                if self.without_time {
                    layer.without_time().boxed()
                } else {
                    layer.boxed()
                }
            }
            OutputFormat::Compact => {
                let layer = base.compact();
                if self.without_time {
                    layer.without_time().boxed()
                } else {
                    layer.boxed()
                }
            }
            OutputFormat::Human => {
                if self.without_time {
                    base.without_time().boxed()
                } else {
                    base.boxed()
                }
            }
        }
    }

    /// Build and globally install the subscriber for this configuration.
    fn install(self) {
        let filter = self.build_filter();
        // Every sink is wrapped in `RedactingWriter`: the Santh safe-defaults
        // contract is that secrets never reach logs, and a redact-only-when-
        // the-caller-remembers default fails open.
        let (writer, ansi): (BoxMakeWriter, bool) = match &self.file_path {
            Some(path) => match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                Ok(file) => (
                    BoxMakeWriter::new(Mutex::new(RedactingWriter::new(file))),
                    false,
                ),
                // file_sink() validated openability; if it has since become
                // unopenable, fall back loudly to stderr rather than panic.
                Err(_) => (
                    BoxMakeWriter::new(Mutex::new(RedactingWriter::new(std::io::stderr()))),
                    true,
                ),
            },
            None => (
                BoxMakeWriter::new(Mutex::new(RedactingWriter::new(std::io::stderr()))),
                true,
            ),
        };
        let layer = self.fmt_layer(writer, ansi);
        // fmt layer at the base (it is typed `Layer<Registry>`); the global
        // EnvFilter layers on top and filters events for the whole stack.
        Registry::default().with(layer).with(filter).init();
    }
}

/// Install the global Santh tracing subscriber from an [`InitConfig`].
///
/// Idempotent via a process-global [`OnceLock`]: the first call installs the
/// subscriber and wins; later calls are no-ops except that they update the
/// process-global tool name (so [`with_op`] spans and metrics are labelled
/// correctly, including from threads that never called `init`).
pub fn init_with(config: InitConfig) -> InitGuard {
    let tool = config.tool.clone();
    // try_init_with installs the subscriber on first call and returns an error
    // on later calls; init_with is idempotent and always updates the tool name.
    let _ = try_init_with(config);
    set_tool_name(tool);
    InitGuard
}

/// Fallible version of [`init_with`].
///
/// Returns [`Error::SubscriberAlreadySet`] if a subscriber has already been
/// installed in this process. Otherwise installs the subscriber and returns
/// the guard.
pub fn try_init_with(config: InitConfig) -> Result<InitGuard, Error> {
    let tool = config.tool.clone();
    let first = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let first_for_closure = first.clone();
    INIT.get_or_init(move || {
        first_for_closure.store(true, std::sync::atomic::Ordering::SeqCst);
        config.install();
    });
    if first.load(std::sync::atomic::Ordering::SeqCst) {
        set_tool_name(tool);
        Ok(InitGuard)
    } else {
        Err(Error::SubscriberAlreadySet)
    }
}

/// The tool name recorded by the most recent [`init`]/[`init_with`] in this
/// process. Used by the Prometheus metrics facade to namespace metric names
/// under `santh_<tool>_`; the no-op facade never reads it, so the function
/// only exists in builds that can call it.
#[cfg(feature = "prometheus")]
pub(crate) fn get_tool_name() -> String {
    TOOL.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Re-export for tool code and tests.
pub mod tracing {
    pub use tracing::*;
}

/// Pops the thread-local operation stack on drop. Using an RAII guard (instead
/// of a manual pop after the body runs) ensures the push is unwound even when
/// `body` panics, so a panicking operation cannot leave a stale entry that
/// pollutes the next task reusing the same thread.
struct OpStackGuard;

impl Drop for OpStackGuard {
    fn drop(&mut self) {
        OP_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Format the op stack into the span's `ops` field: `op="a" op="b"`.
///
/// Extracted (and single-pass) so the formatting is unit-testable and does not
/// allocate one `String` per frame plus a `Vec` plus a `join` on every
/// `with_op` call - it grows a single `String` in one pass instead.
fn format_ops_chain(stack: &[String]) -> String {
    let mut chain = String::new();
    for (index, name) in stack.iter().enumerate() {
        if index > 0 {
            chain.push(' ');
        }
        chain.push_str("op=\"");
        chain.push_str(name);
        chain.push('"');
    }
    chain
}

/// Run `body` inside a nested operation span.
///
/// `op` must be a short, static label (for example `"scan"` or `"parse"`),
/// never user-controlled data. Field values are recorded verbatim into the
/// span; the redacting sink masks secret shapes in the formatted output, but
/// a label built from secrets would still pollute span metadata and metrics
/// namespaces before that point.
pub fn with_op<F, R>(op: &str, body: F) -> R
where
    F: FnOnce() -> R,
{
    OP_STACK.with(|stack| stack.borrow_mut().push(op.to_string()));
    // Balance the push above via RAII: dropped on normal return AND on unwind.
    let _op_guard = OpStackGuard;
    let ops_chain = OP_STACK.with(|stack| format_ops_chain(&stack.borrow()));
    // Record the tool name by BORROW (via the read guard's Display), not by
    // cloning the global `String` on every call. The guard is held only for the
    // span construction and dropped before `body` runs, so `body` can freely
    // re-enter `init`/`set_tool_name` (which take the write lock).
    let span = {
        let tool = TOOL.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        tracing::info_span!(
            parent: tracing::Span::current(),
            "santh_op",
            tool = %*tool,
            op = op,
            ops = %ops_chain
        )
    };
    span.in_scope(body)
}

/// Run `body` inside a span with explicit tool, operation, and target fields.
#[macro_export]
macro_rules! santh_span {
    ($tool:expr, $op:expr, $target:expr, $body:block) => {{
        let span = $crate::tracing::info_span!("santh", tool = $tool, op = $op, target = $target);
        let _enter = span.enter();
        $body
    }};
}

fn level_filter(level: LogLevel) -> EnvFilter {
    use tracing::Level;
    let level = match level {
        LogLevel::Trace => Level::TRACE,
        LogLevel::Debug => Level::DEBUG,
        LogLevel::Info => Level::INFO,
        LogLevel::Warn => Level::WARN,
        LogLevel::Error => Level::ERROR,
    };
    // `Directive: From<Level>` builds the directive directly, with no
    // fallible string round-trip to unwrap.
    EnvFilter::default().add_directive(level.into())
}

#[cfg(test)]
mod build_filter_tests {
    use super::{level_filter, EnvFilter, InitConfig, LogLevel};

    // `build_filter` is the real home of the "env directive wins, else
    // default_filter, else deny->off, else bare level" contract. The
    // tests/adversarial `malformed_rust_log` test could never exercise it: it
    // went through the process-global `init`, which is `OnceLock`-gated, so once
    // a sibling test in the same binary had installed the subscriber this call
    // was a silent no-op and `config.install()` (hence this fallback) never ran
    // - and it asserted nothing anyway. These tests call `build_filter` directly
    // (no global install) and each uses a UNIQUE env-var NAME via `.env_var(..)`
    // instead of the shared `RUST_LOG`, so they read isolated state and are safe
    // to run in parallel with no env-var races.

    #[test]
    fn malformed_env_falls_back_to_the_bare_level() {
        let var = "SANTH_TRACING_TEST_MALFORMED_LEVEL";
        std::env::set_var(var, "invalid=notalevel");
        let filter = InitConfig::new("probe", LogLevel::Warn)
            .env_var(var)
            .build_filter();
        std::env::remove_var(var);

        // No default_filter, deny_by_default=false => malformed env => bare level.
        assert_eq!(
            filter.to_string(),
            level_filter(LogLevel::Warn).to_string(),
            "malformed filter env must fall back to the provided level"
        );
        assert_ne!(
            filter.to_string(),
            level_filter(LogLevel::Error).to_string(),
            "the fallback must be the SPECIFIC provided level (warn), not just any level"
        );
    }

    #[test]
    fn valid_env_directive_wins_over_level() {
        let var = "SANTH_TRACING_TEST_VALID_WINS";
        std::env::set_var(var, "debug");
        let filter = InitConfig::new("probe", LogLevel::Error)
            .env_var(var)
            .build_filter();
        std::env::remove_var(var);
        assert_eq!(
            filter.to_string(),
            EnvFilter::new("debug").to_string(),
            "a valid env directive must win over the bare level"
        );
    }

    #[test]
    fn malformed_env_with_default_filter_uses_default_not_level() {
        let var = "SANTH_TRACING_TEST_DEFAULT_PREC";
        std::env::set_var(var, "invalid=notalevel");
        let filter = InitConfig::new("probe", LogLevel::Error)
            .env_var(var)
            .default_filter("info,hyper=warn")
            .build_filter();
        std::env::remove_var(var);
        assert_eq!(
            filter.to_string(),
            EnvFilter::new("info,hyper=warn").to_string(),
            "default_filter takes precedence over the bare level on malformed env"
        );
    }

    #[test]
    fn malformed_env_with_deny_by_default_is_off() {
        let var = "SANTH_TRACING_TEST_DENY_OFF";
        std::env::set_var(var, "invalid=notalevel");
        let filter = InitConfig::new("probe", LogLevel::Info)
            .env_var(var)
            .deny_by_default()
            .build_filter();
        std::env::remove_var(var);
        assert_eq!(
            filter.to_string(),
            EnvFilter::new("off").to_string(),
            "deny_by_default must silence (off) on malformed env, not fall back to the level"
        );
    }
}

#[cfg(test)]
mod tool_global_tests {
    use super::{set_tool_name, TOOL, TOOL_SERIAL};
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn tool_name_is_visible_from_spawned_threads() {
        // Regression: TOOL used to be thread-local, so metrics and spans emitted
        // from worker threads (which never call `init`) saw an empty name and
        // were namespaced under `santh__`. A process-global must be visible from
        // any thread.
        // Hold the serial lock: mutating the global tool name concurrently with a
        // count-sensitive metrics test would split its counts across two prefixes.
        let _serial = TOOL_SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_tool_name("spawn_probe".to_string());

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let seen = TOOL
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            tx.send(seen).unwrap();
        })
        .join()
        .unwrap();

        assert_eq!(
            rx.recv().unwrap(),
            "spawn_probe",
            "spawned thread must see the process-global tool name, not an empty string"
        );
    }
}

#[cfg(test)]
mod with_op_tests {
    use super::{format_ops_chain, with_op};

    #[test]
    fn format_ops_chain_is_empty_for_no_frames() {
        assert_eq!(format_ops_chain(&[]), "");
    }

    #[test]
    fn format_ops_chain_renders_one_frame_without_a_separator() {
        assert_eq!(format_ops_chain(&["scan".to_string()]), r#"op="scan""#);
    }

    #[test]
    fn format_ops_chain_space_joins_nested_frames_in_order() {
        let stack = vec!["outer".to_string(), "inner".to_string(), "leaf".to_string()];
        assert_eq!(
            format_ops_chain(&stack),
            r#"op="outer" op="inner" op="leaf""#,
            "frames must render outermost-first, space separated, each quoted once"
        );
    }

    #[test]
    fn with_op_returns_the_body_value_and_pops_the_stack() {
        // The op stack must be balanced after with_op returns (RAII pop), and the
        // body's value is passed through unchanged.
        let value = with_op("unit", || 41 + 1);
        assert_eq!(value, 42);
        // A subsequent op sees a clean stack: its chain is exactly one frame.
        with_op("after", || {
            let chain = super::OP_STACK.with(|stack| format_ops_chain(&stack.borrow()));
            assert_eq!(chain, r#"op="after""#, "stack must be balanced between calls");
        });
    }
}
