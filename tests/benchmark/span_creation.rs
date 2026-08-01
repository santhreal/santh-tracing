// Performance contract: span creation must complete in less than 1 microsecond.

use criterion::{criterion_group, criterion_main, Criterion};

fn span_creation(c: &mut Criterion) {
    c.bench_function("santh_span_new", |b| {
        b.iter(|| {
            // The span shape `santh_span!` produces: a "santh" span carrying
            // tool/op/target. Benchmark the creation cost via the canonical
            // span macro directly - no redundant wrapper type.
            let span = santh_tracing::tracing::info_span!(
                "santh",
                tool = "tool",
                op = "op",
                target = "target"
            );
            criterion::black_box(span);
        });
    });
}

criterion_group!(benches, span_creation);
criterion_main!(benches);
