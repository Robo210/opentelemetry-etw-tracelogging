use opentelemetry::trace::TraceContextExt;
use opentelemetry::Key;
use opentelemetry_api::global::shutdown_tracer_provider;
use opentelemetry_api::trace::{Span, Tracer};
use opentelemetry_etw as otel_etw;

const SAMPLE_KEY_STR: Key = Key::from_static_str("str");
const SAMPLE_KEY_BOOL: Key = Key::from_static_str("bool");
const SAMPLE_KEY_INT: Key = Key::from_static_str("int");
const SAMPLE_KEY_FLOAT: Key = Key::from_static_str("float");

fn main() {
    let tracer = otel_etw::span_exporter::new_etw_exporter("Sample-Provider-Name").install_simple();

    tracer.in_span("OuterSpanName", |cx| {
        std::thread::sleep(std::time::Duration::from_millis(1000));

        let span = cx.span();
        span.add_event(
            "SampleEventName",
            vec![SAMPLE_KEY_STR.string("sample string"), SAMPLE_KEY_BOOL.bool(true)],
        );

        let span_builder = tracer
            .span_builder("OuterSpanName")
            .with_kind(opentelemetry::trace::SpanKind::Client)
            .with_status(opentelemetry::trace::Status::Error {
                description: "My error message".into(),
            });

        let mut span = tracer.build(span_builder);

        std::thread::sleep(std::time::Duration::from_millis(1000));
        span.add_event("SampleEvent2", vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1000));
    });

    std::thread::sleep(std::time::Duration::from_millis(1000));

    let tracer2 = otel_etw::span_exporter::new_etw_exporter("Sample-Provider-Name").install_realtime();

    tracer2.in_span("RealtimeOuterSpanName", |cx| {
        std::thread::sleep(std::time::Duration::from_millis(1000));

        let span = cx.span();
        span.add_event(
            "RealtimeSampleEventName",
            vec![SAMPLE_KEY_INT.i64(5), SAMPLE_KEY_FLOAT.f64(7.1)],
        );

        let span_builder = tracer2
            .span_builder("RealtimeOuterSpanName")
            .with_kind(opentelemetry::trace::SpanKind::Server)
            .with_status(opentelemetry::trace::Status::Ok);

        let mut span = tracer2.build(span_builder);

        std::thread::sleep(std::time::Duration::from_millis(1000));
        span.add_event("RealtimeSampleEvent2", vec![SAMPLE_KEY_BOOL.array(vec![false, true, false])]);
        std::thread::sleep(std::time::Duration::from_millis(1000));
    });

    shutdown_tracer_provider(); // sending remaining spans
}
