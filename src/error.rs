use std::fmt;

/// Errors that can occur in `santh-tracing`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The global tracing subscriber is already installed.
    SubscriberAlreadySet,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::SubscriberAlreadySet => write!(
                f,
                "the global tracing subscriber is already installed; fix: call `santh_tracing::init` exactly once per process"
            ),
        }
    }
}

impl std::error::Error for Error {}
