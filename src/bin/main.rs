use opentelemetry::trace::TraceContextExt;
use opentelemetry::Key;
use opentelemetry_api::global::shutdown_tracer_provider;
use opentelemetry_api::trace::{Span, Tracer};

const KYLE_KEY: Key = Key::from_static_str("kylesabo.com/foo");
const SABO_KEY: Key = Key::from_static_str("kylesabo.com/bar");

fn main() {
    let tracer = opentelemetry_etw_tracelogging::new_pipeline()
        .with_name("kyle")
        .with_keyword(2)
        .install_simple();

    tracer.in_span("doing_work", |cx| {
        std::thread::sleep(std::time::Duration::from_millis(1000));

        let span = cx.span();
        span.add_event(
            "sabo",
            vec![KYLE_KEY.string("is cool"), SABO_KEY.string("is great")],
        );
    });

    let span_builder = tracer
        .span_builder("my_cool_span")
        .with_kind(opentelemetry::trace::SpanKind::Client)
        .with_status(opentelemetry::trace::Status::Error {
            description: "asdf".into(),
        });
    let mut span = span_builder.start(&tracer);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    span.add_event("qwerty", vec![]);
    std::thread::sleep(std::time::Duration::from_millis(1000));

    drop(span);
    shutdown_tracer_provider(); // sending remaining spans
}
