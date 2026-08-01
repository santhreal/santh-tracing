//! Metrics facade that automatically namespaces under `santh_<tool_name>_`.

#[cfg(not(feature = "prometheus"))]
mod noop;
#[cfg(not(feature = "prometheus"))]
pub use noop::*;

#[cfg(feature = "prometheus")]
mod prom;
#[cfg(feature = "prometheus")]
pub use prom::*;
