use opentelemetry::trace::TraceContextExt;
use opentelemetry::Key;
use opentelemetry_api::global::shutdown_tracer_provider;
use opentelemetry_api::trace::Tracer;

const KYLE_KEY: Key = Key::from_static_str("kylesabo.com/foo");
const SABO_KEY: Key = Key::from_static_str("kylesabo.com/bar");

fn main() {
    let tracer = opentelemetry_etw_tracelogging::new_pipeline()
        .with_name("kyle")
        .install_simple();
    tracer.in_span("doing_work", |cx| {
        std::thread::sleep(std::time::Duration::from_millis(1000));

        let span = cx.span();
        span.add_event(
            "sabo",
            vec![KYLE_KEY.string("is cool"), SABO_KEY.string("is great")],
        );
    });
    shutdown_tracer_provider(); // sending remaining spans
}
