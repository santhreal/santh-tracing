//! Prometheus-backed metrics implementation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use prometheus::{CounterVec, GaugeVec, HistogramOpts, HistogramVec, Opts};

use crate::get_tool_name;

#[derive(Clone)]
enum MetricVec {
    Counter(CounterVec),
    Histogram(HistogramVec),
    Gauge(GaugeVec),
}

/// Which metric flavor a lookup wants; part of the cache key so a name reused
/// across flavors cannot cross-resolve.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Counter,
    Histogram,
    Gauge,
}

static METRICS: OnceLock<Mutex<HashMap<String, MetricVec>>> = OnceLock::new();

/// Per-thread cache of resolved metric vectors, keyed by a hash of
/// `(kind, name, label-names)` and validated on hit. A hit skips the whole
/// allocating slow path (`get_tool_name` clone, `format!`, `sanitize_metric_name`,
/// the label-name `Vec`, the composed key `String`, and the global mutex),
/// returning a cheap `Arc`-clone of the vector.
struct LocalCache {
    generation: u64,
    entries: HashMap<u64, CachedMetric>,
}

struct CachedMetric {
    kind: Kind,
    name: String,
    label_names: Vec<String>,
    metric: MetricVec,
}

thread_local! {
    static LOCAL_CACHE: RefCell<LocalCache> = RefCell::new(LocalCache {
        // u64::MAX forces a generation-mismatch (hence a clear + sync) on the
        // very first lookup, regardless of the starting TOOL_GENERATION.
        generation: u64::MAX,
        entries: HashMap::new(),
    });
}

// Per-thread count of slow-path resolutions (cache misses). Thread-local (not a
// global atomic) so a proving test's delta measures only ITS OWN thread's
// resolutions and is not perturbed by other tests resolving metrics in parallel.
// The proving test asserts warmed cached lookups do NOT increment it, i.e. the
// allocating construction path is skipped on a hit. Compiled out of release.
#[cfg(test)]
thread_local! {
    static SLOW_PATH_RESOLUTIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_slow_path() {
    SLOW_PATH_RESOLUTIONS.with(|c| c.set(c.get() + 1));
}

#[cfg(test)]
fn slow_path_count() -> u64 {
    SLOW_PATH_RESOLUTIONS.with(std::cell::Cell::get)
}

#[cfg(not(test))]
#[inline(always)]
fn record_slow_path() {}

fn hash_metric_key(kind: Kind, name: &str, labels: &[(&str, &str)]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (kind as u8).hash(&mut hasher);
    name.hash(&mut hasher);
    for (key, _) in labels {
        key.hash(&mut hasher);
    }
    hasher.finish()
}

/// True when a cached entry's stored label names equal this call's label keys
/// (order-sensitive, matching how the metric vector was registered). Borrow-only,
/// no allocation.
fn label_names_match(stored: &[String], labels: &[(&str, &str)]) -> bool {
    stored.len() == labels.len() && stored.iter().zip(labels).all(|(s, (k, _))| s == k)
}

/// Resolve a metric vector, using the per-thread cache when possible and falling
/// back to the global create-and-register path (the only allocating branch) on a
/// miss. The returned `MetricVec` variant always matches `kind`.
fn resolve(kind: Kind, name: &str, labels: &[(&str, &str)]) -> Option<MetricVec> {
    let generation = crate::tool_generation();
    let hash = hash_metric_key(kind, name, labels);

    let cached = LOCAL_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.generation != generation {
            // Tool name changed: every cached full-name is now mis-namespaced.
            cache.entries.clear();
            cache.generation = generation;
        }
        cache.entries.get(&hash).and_then(|entry| {
            if entry.kind == kind
                && entry.name == name
                && label_names_match(&entry.label_names, labels)
            {
                Some(entry.metric.clone())
            } else {
                None
            }
        })
    });
    if let Some(metric) = cached {
        return Some(metric);
    }

    // Slow path (cache miss): this is the only branch that allocates.
    record_slow_path();
    let tool = get_tool_name();
    let full_name = sanitize_metric_name(&format!("santh_{tool}_{name}"));
    let metric = match kind {
        Kind::Counter => MetricVec::Counter(get_or_create_counter(&full_name, labels)?),
        Kind::Histogram => MetricVec::Histogram(get_or_create_histogram(&full_name, labels)?),
        Kind::Gauge => MetricVec::Gauge(get_or_create_gauge(&full_name, labels)?),
    };
    LOCAL_CACHE.with(|cell| {
        cell.borrow_mut().entries.insert(
            hash,
            CachedMetric {
                kind,
                name: name.to_owned(),
                label_names: labels.iter().map(|(k, _)| (*k).to_owned()).collect(),
                metric: metric.clone(),
            },
        );
    });
    Some(metric)
}

/// Invoke `f` with the label VALUES as a `&[&str]`, using a stack buffer for the
/// common small-cardinality case so a warmed record allocates nothing. Falls
/// back to a heap `Vec` only past the stack bound.
fn with_label_values<R>(labels: &[(&str, &str)], f: impl FnOnce(&[&str]) -> R) -> R {
    const STACK_LABELS: usize = 12;
    if labels.len() <= STACK_LABELS {
        let mut buf = [""; STACK_LABELS];
        for (slot, (_, value)) in buf.iter_mut().zip(labels) {
            *slot = value;
        }
        f(&buf[..labels.len()])
    } else {
        let values: Vec<&str> = labels.iter().map(|(_, value)| *value).collect();
        f(&values)
    }
}

/// Coerce a metric name into the Prometheus charset `[a-zA-Z0-9_:]`, replacing
/// every other byte with `_`.
///
/// Tool names carry hyphens (`santh-cli`), which Prometheus rejects; without
/// this, `CounterVec::new` would return `Err` and the metric would be silently
/// never created or exported for every hyphenated tool. Names always start with
/// the `santh_` prefix, so the leading-character rule is already satisfied.
fn sanitize_metric_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == ':' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn metric_key(name: &str, label_names: &[&str]) -> String {
    let mut key = name.to_owned();
    for ln in label_names {
        key.push('|');
        key.push_str(ln);
    }
    key
}

fn get_or_create_counter(name: &str, labels: &[(&str, &str)]) -> Option<CounterVec> {
    let map = METRICS.get_or_init(|| Mutex::new(HashMap::new()));
    let label_names: Vec<&str> = labels.iter().map(|(k, _)| *k).collect();
    let key = metric_key(name, &label_names);

    // Hold the lock across check+create+register+insert. Dropping it between the
    // check and the insert let two threads both create the metric, both call
    // register() (the second failing AlreadyRegistered), and both insert - the
    // loser overwriting the registered vec with an UNREGISTERED one whose samples
    // are then silently never exported.
    // Recover a poisoned lock (a thread panicked mid-update) rather than
    // silently dropping every future metric via `.ok()?` (Law 10). The map is a
    // plain HashMap cache whose invariants survive a mid-update panic.
    let mut m = map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(MetricVec::Counter(vec)) = m.get(&key) {
        return Some(vec.clone());
    }
    let opts = Opts::new(name, format!("santh counter {name}"));
    let vec = match CounterVec::new(opts, &label_names) {
        Ok(v) => v,
        Err(e) => {
            // Not silent (Law 10): an invalid name/label set means this metric
            // can never be created or exported. Surface it loudly instead of
            // returning a no-op that hides the misconfiguration forever.
            tracing::warn!(metric = %name, error = %e, "prometheus counter construction failed; metric will not be created");
            return None;
        }
    };
    if let Err(e) = prometheus::default_registry().register(Box::new(vec.clone())) {
        // Not silent (Law 10): a failed registration means this vec's samples
        // are never exported. The lock above prevents our own double-register,
        // so this fires only for an external name collision or a bad metric.
        tracing::warn!(
            metric = %name,
            error = %e,
            "prometheus metric registration failed; samples will not export"
        );
    }
    m.insert(key, MetricVec::Counter(vec.clone()));
    Some(vec)
}

fn get_or_create_histogram(name: &str, labels: &[(&str, &str)]) -> Option<HistogramVec> {
    let map = METRICS.get_or_init(|| Mutex::new(HashMap::new()));
    let label_names: Vec<&str> = labels.iter().map(|(k, _)| *k).collect();
    let key = metric_key(name, &label_names);

    // See get_or_create_counter: the lock is held across check+create+register+
    // insert to prevent a concurrent creator from inserting an unregistered vec.
    // Recover a poisoned lock (a thread panicked mid-update) rather than
    // silently dropping every future metric via `.ok()?` (Law 10). The map is a
    // plain HashMap cache whose invariants survive a mid-update panic.
    let mut m = map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(MetricVec::Histogram(vec)) = m.get(&key) {
        return Some(vec.clone());
    }
    let opts = HistogramOpts::new(name, format!("santh histogram {name}"));
    let vec = match HistogramVec::new(opts, &label_names) {
        Ok(v) => v,
        Err(e) => {
            // Not silent (Law 10): see get_or_create_counter.
            tracing::warn!(metric = %name, error = %e, "prometheus histogram construction failed; metric will not be created");
            return None;
        }
    };
    if let Err(e) = prometheus::default_registry().register(Box::new(vec.clone())) {
        // Not silent (Law 10): a failed registration means this vec's samples
        // are never exported. The lock above prevents our own double-register,
        // so this fires only for an external name collision or a bad metric.
        tracing::warn!(
            metric = %name,
            error = %e,
            "prometheus metric registration failed; samples will not export"
        );
    }
    m.insert(key, MetricVec::Histogram(vec.clone()));
    Some(vec)
}

fn get_or_create_gauge(name: &str, labels: &[(&str, &str)]) -> Option<GaugeVec> {
    let map = METRICS.get_or_init(|| Mutex::new(HashMap::new()));
    let label_names: Vec<&str> = labels.iter().map(|(k, _)| *k).collect();
    let key = metric_key(name, &label_names);

    // See get_or_create_counter: the lock is held across check+create+register+
    // insert to prevent a concurrent creator from inserting an unregistered vec.
    // Recover a poisoned lock (a thread panicked mid-update) rather than
    // silently dropping every future metric via `.ok()?` (Law 10). The map is a
    // plain HashMap cache whose invariants survive a mid-update panic.
    let mut m = map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(MetricVec::Gauge(vec)) = m.get(&key) {
        return Some(vec.clone());
    }
    let opts = Opts::new(name, format!("santh gauge {name}"));
    let vec = match GaugeVec::new(opts, &label_names) {
        Ok(v) => v,
        Err(e) => {
            // Not silent (Law 10): see get_or_create_counter.
            tracing::warn!(metric = %name, error = %e, "prometheus gauge construction failed; metric will not be created");
            return None;
        }
    };
    if let Err(e) = prometheus::default_registry().register(Box::new(vec.clone())) {
        // Not silent (Law 10): a failed registration means this vec's samples
        // are never exported. The lock above prevents our own double-register,
        // so this fires only for an external name collision or a bad metric.
        tracing::warn!(
            metric = %name,
            error = %e,
            "prometheus metric registration failed; samples will not export"
        );
    }
    m.insert(key, MetricVec::Gauge(vec.clone()));
    Some(vec)
}

/// Increment a counter metric.
///
/// Metrics are automatically namespaced under `santh_<tool_name>_<name>`.
pub fn counter(name: &str, labels: &[(&str, &str)]) {
    if let Some(MetricVec::Counter(vec)) = resolve(Kind::Counter, name, labels) {
        with_label_values(labels, |values| match vec.get_metric_with_label_values(values) {
            Ok(metric) => metric.inc(),
            Err(e) => tracing::warn!(metric = %name, error = %e, "failed to get counter metric with label values"),
        });
    }
}

/// Record a histogram observation.
pub fn histogram(name: &str, value: f64, labels: &[(&str, &str)]) {
    if let Some(MetricVec::Histogram(vec)) = resolve(Kind::Histogram, name, labels) {
        with_label_values(labels, |values| match vec.get_metric_with_label_values(values) {
            Ok(metric) => metric.observe(value),
            Err(e) => tracing::warn!(metric = %name, error = %e, "failed to get histogram metric with label values"),
        });
    }
}

/// Set a gauge value.
pub fn gauge(name: &str, value: f64, labels: &[(&str, &str)]) {
    if let Some(MetricVec::Gauge(vec)) = resolve(Kind::Gauge, name, labels) {
        with_label_values(labels, |values| match vec.get_metric_with_label_values(values) {
            Ok(metric) => metric.set(value),
            Err(e) => tracing::warn!(metric = %name, error = %e, "failed to get gauge metric with label values"),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn concurrent_first_use_lands_all_increments_on_the_registered_counter() {
        // Regression: concurrent first-use of the same metric used to create and
        // register duplicate vecs (the 2nd register failing AlreadyRegistered),
        // leaving an UNREGISTERED vec in the map whose increments never reach the
        // registry export. Every increment must be exported exactly once.
        const THREADS: usize = 64;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let b = Arc::clone(&barrier);
                thread::spawn(move || {
                    b.wait();
                    super::counter("race_probe_total", &[]);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        // Read the exported value via the text exposition format, which is
        // stable across prometheus/protobuf versions.
        let mut buf = String::new();
        prometheus::TextEncoder::new()
            .encode_utf8(&prometheus::gather(), &mut buf)
            .expect("encode metrics");
        let line = buf
            .lines()
            .find(|l| !l.starts_with('#') && l.contains("race_probe_total"))
            .expect("counter must be registered and exported exactly once");
        let value: f64 = line
            .rsplit(' ')
            .next()
            .and_then(|v| v.parse().ok())
            .expect("counter line must end in a numeric value");
        assert!(
            (value - THREADS as f64).abs() < f64::EPSILON,
            "all {THREADS} increments must land on the single registered counter, not a lost duplicate"
        );
    }

    #[test]
    fn sanitize_metric_name_coerces_invalid_chars() {
        // Hyphens (from tool names like `santh-cli`) and dots become `_`; `:` and
        // alphanumerics/underscore are preserved.
        assert_eq!(
            super::sanitize_metric_name("santh_santh-cli_scans_total"),
            "santh_santh_cli_scans_total"
        );
        assert_eq!(super::sanitize_metric_name("a.b-c:d_e"), "a_b_c:d_e");
        assert_eq!(
            super::sanitize_metric_name("already_valid_1"),
            "already_valid_1"
        );
    }

    #[test]
    fn counter_recovers_from_a_poisoned_metrics_lock() {
        // Prime the map so METRICS is initialized, then poison its lock by
        // panicking while holding the guard. A metric recorded afterwards must
        // still be created and exported: the old `map.lock().ok()?` would return
        // None on a poisoned lock and silently drop every future metric (Law 10).
        super::counter("poison_probe_warmup_total", &[]);
        let map = super::METRICS.get().expect("metrics map initialized");

        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = map.lock().unwrap();
            panic!("poison the metrics map");
        }));
        assert!(poisoned.is_err(), "poisoning closure must have panicked");
        assert!(map.is_poisoned(), "metrics lock must be poisoned");

        super::counter("poison_recovery_probe_total", &[]);

        let mut buf = String::new();
        prometheus::TextEncoder::new()
            .encode_utf8(&prometheus::gather(), &mut buf)
            .expect("encode metrics");
        assert!(
            buf.lines()
                .any(|l| !l.starts_with('#') && l.contains("poison_recovery_probe_total")),
            "counter recorded after lock poisoning must still export:\n{buf}"
        );
    }

    #[test]
    fn invalid_label_metric_is_contained_not_corrupting() {
        let _serial = crate::TOOL_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A hyphenated label KEY (label keys are not sanitized) makes
        // CounterVec::new fail. The construction failure must be contained: the
        // call is a safe no-op (metric not created/exported) and does NOT panic
        // or prevent a subsequent valid metric from working. The fix also emits a
        // tracing::warn! so the failure is not silent (Law 10).
        super::counter("valid_after_bad_probe_total", &[]);
        // Invalid label key: must not panic, must not create a metric.
        super::counter("bad_label_probe_total", &[("bad-key", "v")]);
        super::counter("valid_after_bad_probe_total", &[]);

        let mut buf = String::new();
        prometheus::TextEncoder::new()
            .encode_utf8(&prometheus::gather(), &mut buf)
            .expect("encode metrics");
        // The invalid-label metric was never created.
        assert!(
            !buf.contains("bad_label_probe_total"),
            "a metric with an invalid label must not be created:\n{buf}"
        );
        // The valid metric still works and accumulated both increments.
        let line = buf
            .lines()
            .find(|l| !l.starts_with('#') && l.contains("valid_after_bad_probe_total"))
            .expect("the valid metric must still export");
        let value: f64 = line
            .rsplit(' ')
            .next()
            .and_then(|v| v.parse().ok())
            .expect("counter line ends in a numeric value");
        assert!((value - 2.0).abs() < f64::EPSILON, "both valid increments must land: {line}");
    }

    #[test]
    fn warmed_cached_lookup_skips_the_allocating_slow_path() {
        // Hold the serial lock so no test mutates the global tool name during our
        // run (which would bump the generation, invalidate our cache mid-loop, and
        // split our counts across two tool prefixes).
        let _serial = crate::TOOL_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Warm the metric: first call creates+registers it and populates the
        // per-thread cache; a second call proves the cache is now hot.
        super::counter("cache_probe_total", &[("phase", "warm")]);
        super::counter("cache_probe_total", &[("phase", "warm")]);

        // Baseline the (thread-local) slow-path counter, then record the SAME
        // metric many more times. A warmed cache must serve every one without
        // re-entering the allocating construction path (get_tool_name clone,
        // format!, sanitize, label-name Vec, key String, global mutex).
        let before = super::slow_path_count();
        for _ in 0..1000 {
            super::counter("cache_probe_total", &[("phase", "warm")]);
        }
        let after = super::slow_path_count();
        assert_eq!(
            after, before,
            "warmed cached lookups must not re-enter the slow allocating path"
        );

        // A DIFFERENT metric name is a distinct cache entry: it must miss exactly
        // once, then serve from cache. (Note: the SAME name+label-NAMES with a
        // different label VALUE correctly reuses one vector - the vector is keyed
        // by name+label-names and the value only selects the child.)
        let before_new = super::slow_path_count();
        super::counter("cache_probe_second_total", &[("phase", "warm")]);
        let mid_new = super::slow_path_count();
        super::counter("cache_probe_second_total", &[("phase", "warm")]);
        let after_new = super::slow_path_count();
        assert_eq!(
            mid_new - before_new,
            1,
            "a new metric must take the slow path exactly once"
        );
        assert_eq!(
            after_new - mid_new,
            0,
            "the second call to the new metric must serve from cache"
        );

        // Every warm increment must still be exported: the cache must not drop or
        // misroute samples. Sum across any tool prefixes present, so the assertion
        // is exact even if the tool name changed at some earlier point.
        let mut buf = String::new();
        prometheus::TextEncoder::new()
            .encode_utf8(&prometheus::gather(), &mut buf)
            .expect("encode metrics");
        let warm_total: f64 = buf
            .lines()
            .filter(|l| {
                !l.starts_with('#') && l.contains("cache_probe_total") && l.contains("warm")
            })
            .filter_map(|l| l.rsplit(' ').next().and_then(|v| v.parse::<f64>().ok()))
            .sum();
        // 2 warm + 1000 loop increments all land on the cached vector(s).
        assert!((warm_total - 1002.0).abs() < f64::EPSILON, "all warm increments must export:\n{buf}");
    }

    #[test]
    fn a_generation_change_clears_the_cache_and_forces_reresolution() {
        // Hold the serial lock so no concurrent set_tool_name bumps the GLOBAL
        // generation mid-test (which would clear our cache between the two final
        // calls and defeat the "cached again" assertion).
        let _serial = crate::TOOL_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Deterministically exercise generation invalidation: populate this
        // thread's cache, then desync its recorded generation (what a real
        // set_tool_name bump does) and confirm the next lookup clears the cache
        // and re-resolves exactly once, then re-caches.
        super::counter("gen_clear_probe_total", &[]);
        super::counter("gen_clear_probe_total", &[]); // cached

        super::LOCAL_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            assert!(!cache.entries.is_empty(), "cache must hold the warmed entry");
            cache.generation = cache.generation.wrapping_add(1); // simulate a tool change
        });

        let before = super::slow_path_count();
        super::counter("gen_clear_probe_total", &[]); // generation mismatch -> clear + miss
        assert_eq!(
            super::slow_path_count() - before,
            1,
            "a generation change must clear the cache and force one re-resolve"
        );

        let mid = super::slow_path_count();
        super::counter("gen_clear_probe_total", &[]); // re-resolved entry is cached again
        assert_eq!(
            super::slow_path_count() - mid,
            0,
            "after re-resolution the entry must be cached again"
        );
    }

    #[test]
    fn set_tool_name_bumps_the_generation() {
        // The real invalidation trigger: every tool-name write bumps the
        // generation. Read the current name and set it back so the effective tool
        // is unchanged (no prefix corruption for parallel tests), then assert the
        // generation strictly advanced.
        let _serial = crate::TOOL_SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = crate::get_tool_name();
        let before = crate::tool_generation();
        crate::set_tool_name(current);
        assert!(
            crate::tool_generation() > before,
            "set_tool_name must bump the generation so metric caches invalidate"
        );
    }

    #[test]
    fn hyphenated_metric_name_is_sanitized_and_exported() {
        // A hyphen anywhere in the composed name would make prometheus reject the
        // vec, silently dropping the metric. Sanitization must coerce it to a
        // valid name that actually registers and exports.
        super::counter("hyphen-probe-total", &[]);

        let mut buf = String::new();
        prometheus::TextEncoder::new()
            .encode_utf8(&prometheus::gather(), &mut buf)
            .expect("encode metrics");
        assert!(
            buf.lines()
                .any(|l| !l.starts_with('#') && l.contains("hyphen_probe_total")),
            "hyphenated metric name must be sanitized and exported:\n{buf}"
        );
        assert!(
            !buf.contains("hyphen-probe-total"),
            "the raw hyphenated name must never reach the export"
        );
    }
}
