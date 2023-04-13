use opentelemetry::trace::{Link, TraceContextExt};
use opentelemetry::Key;
use opentelemetry_api::global::shutdown_tracer_provider;
use opentelemetry_api::trace::{Span, Tracer};
use opentelemetry_etw as otel_etw;
use otel_etw::span_exporter::EtwExporterAsyncRuntime;

const SAMPLE_KEY_STR: Key = Key::from_static_str("str");
const SAMPLE_KEY_BOOL: Key = Key::from_static_str("bool");
const SAMPLE_KEY_INT: Key = Key::from_static_str("int");
const SAMPLE_KEY_FLOAT: Key = Key::from_static_str("float");

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tracer2 = otel_etw::span_exporter::new_etw_exporter("SampleProviderName")
            .with_common_schema_events()
            .without_realtime_events()
            .with_async_runtime(EtwExporterAsyncRuntime::Tokio)
            .install();

        tracer2.in_span("OuterSpanName", |cx| {
            std::thread::sleep(std::time::Duration::from_millis(1000));

            let mut span = cx.span();
            span.set_attributes(vec![SAMPLE_KEY_INT.i64(5), SAMPLE_KEY_FLOAT.f64(7.1)]);

            span.add_event(
                "SampleEventName",
                vec![SAMPLE_KEY_INT.i64(5), SAMPLE_KEY_FLOAT.f64(7.1)],
            );

            let link = Link::new(span.span_context().clone(), vec![SAMPLE_KEY_STR.string("link_attribute")]);

            let span_builder = tracer2
                .span_builder("InnerSpanName")
                .with_kind(opentelemetry::trace::SpanKind::Server)
                .with_links(vec![link])
                .with_status(opentelemetry::trace::Status::Ok);

            let mut span = tracer2.build(span_builder);

            std::thread::sleep(std::time::Duration::from_millis(1000));
            span.add_event(
                "SampleEventName2",
                vec![SAMPLE_KEY_BOOL.array(vec![false, true, false])],
            );

            std::thread::sleep(std::time::Duration::from_millis(1000));
        });

        shutdown_tracer_provider(); // sending remaining spans
    })
}
