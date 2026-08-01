//! No-op metrics implementation used when the `prometheus` feature is
//! disabled.

/// Increment a counter metric.
///
/// Metrics are namespaced under `santh_<tool_name>_<name>`.
pub fn counter(_name: &str, _labels: &[(&str, &str)]) {}

/// Record a histogram observation.
pub fn histogram(_name: &str, _value: f64, _labels: &[(&str, &str)]) {}

/// Set a gauge value.
pub fn gauge(_name: &str, _value: f64, _labels: &[(&str, &str)]) {}
