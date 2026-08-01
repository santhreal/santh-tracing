//! Shared test helpers.
//!
//! Included into several test binaries via `#[path] mod support;`; each binary
//! uses only the subset it needs, so unused-item warnings here are expected.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use tracing::Dispatch;
use tracing_subscriber::{layer::SubscriberExt, Registry};

/// Capture all tracing output produced by `f` and return it as a string.
pub fn capture_output<F>(f: F) -> String
where
    F: FnOnce(),
{
    let buf = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = Arc::clone(&buf);

    let layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        // Disable ANSI so captured output is plain text and substring
        // assertions (e.g. `tool="x"`) are not split by colour escapes.
        .with_ansi(false)
        .with_writer(move || TestWriter(Arc::clone(&buf_clone)));

    let subscriber = Registry::default().with(layer);
    let dispatch = Dispatch::new(subscriber);

    tracing::dispatcher::with_default(&dispatch, f);

    let bytes = buf.lock().expect("poisoned").clone();
    String::from_utf8(bytes).expect("invalid utf8")
}

struct TestWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TestWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Lock to serialize tests that mutate global init state.
pub static INIT_LOCK: Mutex<()> = Mutex::new(());
