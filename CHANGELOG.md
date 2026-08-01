# Changelog

All notable changes to this crate are documented here, following
[Keep a Changelog](https://keepachangelog.com/) and semantic versioning.

## 0.2.1

### Security

- **Default sinks now redact secrets.** Both the stderr sink and the
  `file_sink` sink run through `RedactingWriter`, closing a fail-open default
  where a tool that did not wrap its writer wrote secrets to its logs in the
  clear. The previously `#[ignore]`d gap test
  `default_file_sink_redacts_secrets` now runs green.

### Fixed

- `RedactingWriter` no longer constructs its PEM markers through `Regex::new`
  with `expect`; begin/end markers are constant strings matched by substring
  search (leftmost match preserved), clearing the crate's deny-level
  `clippy::expect_used` violations. The `regex` dependency is removed.
- `flush_secret_region` gained a defensive branch for a tracked opener at or
  past the flush cut point, so `cut - start` can never underflow if a future
  caller changes the buffer invariant.
- `Error::SubscriberAlreadySet` now states what went wrong alongside the fix.
- `with_op` documents that `op` must be a static label, never user-controlled
  data.

## 0.2.0

### Added

- `InitConfig` builder and `init_with` for declarative subscriber setup:
  `json_output`, `compact`, `without_time`, `default_filter`, `env_var`,
  `file_sink`, and `deny_by_default`.
- `RedactingWriter`: a `std::io::Write` adapter that masks secrets
  line-by-line through `santh-error`'s canonical redactor before output
  reaches a sink.
- `prometheus` feature: swaps the no-op metrics facade for a Prometheus-backed
  one. Off by default; metrics are no-ops unless enabled.

### Changed

- Logs are written to **stderr** (the Santh logging contract), so
  machine-readable findings on stdout are never corrupted.
- `metrics` is now a public module.
- `init(tool, level)` is defined as `init_with(InitConfig::new(tool, level))`.
- `level_filter` builds its directive via `Directive: From<Level>` instead of a
  fallible string round-trip.

## 0.1.0

- Initial release: `init`, `InitGuard`, `with_op`, the `santh_span!` macro, and
  the `metrics` facade.
