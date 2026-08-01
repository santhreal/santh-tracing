# santh-tracing SPEC

## Architecture

`santh-tracing` is a thin wrapper around `tracing` and `tracing-subscriber`.
It installs a single global subscriber that produces consistent log shape
across all Santh tools.

## Span Shape

Every span carries three optional fields:

- `tool`  -  static tool name passed to `init`
- `op`  -  operation name
- `target`  -  scan target / URL / file path

Empty fields are omitted from output.

## Log Format

- Human-readable when stdout is a TTY.
- JSON when stdout is not a TTY.
- `EnvFilter` respecting `RUST_LOG` > `level` argument > `info`.

## Metrics

The `metrics` module exposes `counter`, `histogram`, and `gauge`.
All metric names are automatically prefixed with `santh_<tool_name>_`.

When the `prometheus` feature is enabled, metrics register with the global
Prometheus registry. Otherwise they are no-ops.

## Gaps

- OTLP export is a future feature.
- No async-specific span helpers yet.
