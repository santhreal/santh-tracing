# santh-tracing

[![status: beta](https://img.shields.io/badge/status-beta-orange.svg)](https://santh.dev)

> **Internal Santh tooling.** This crate is the shared logging setup for Santh
> CLI tools, not a public library. It is not published to crates.io and its
> API carries no stability guarantee for external users. If you are building
> something outside Santh, configure `tracing-subscriber` directly.

## What it does

A consistent tracing setup for Santh CLI tools. One `init` call (or an
`InitConfig` builder for JSON, compact, file-sink, or deny-by-default modes)
installs a subscriber that logs to stderr by default, keeping machine-readable
findings on stdout uncorrupted. It adds operation spans through `with_op`, an
optional `RedactingWriter` that masks secrets line-by-line, and a metrics facade
that is a no-op until the `prometheus` feature is enabled.

## Quick start

```rust
use santh_tracing::{init, LogLevel};

let _guard = init("mytool", LogLevel::Info);
santh_tracing::tracing::info!("ready");
```

## When to use / When not

Use it as the single tracing entry point for any Santh CLI, so every tool shares
the same span shape, stderr discipline, and redaction story.

Do not use it inside a library crate that should stay subscriber-agnostic, and
do not reach past it to `tracing_subscriber` directly; extend `InitConfig`
instead so every tool gains the improvement at once.

## Compared to alternatives

Calling `tracing_subscriber` directly in every binary works but drifts: each
tool re-derives filtering, output format, and the stdout/stderr split slightly
differently. `tracing-bunyan-formatter` gives JSON but takes no position on
redaction or the findings/logs split. `santh-tracing` centralises those
decisions: stderr by default, a `RUST_LOG`-or-level filter, a shared secret
redactor, and one builder every tool configures the same way.

## How it fits in Santh

`santh-tracing` sits in the `libs/general` layer, just above `santh-error`,
whose redactor it reuses. CLI crates depend on it (often through `santh-cli`)
for logging and metrics. It depends on no higher layer.

## Contributing

Add new output shapes as `InitConfig` builder methods, not as direct
`tracing_subscriber` calls in tools. Cover each mode with an integration test in
`tests/` and keep the stderr-by-default and redaction guarantees intact.

## License

Licensed under either MIT or Apache-2.0.
