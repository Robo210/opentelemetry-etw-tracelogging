use opentelemetry_api::global::shutdown_tracer_provider;
use opentelemetry_api::trace::Tracer;
use opentelemetry_etw_tracelogging::*;

fn main() {
    let tracer = opentelemetry_etw_tracelogging::new_pipeline()
        .with_name("kyle")
        .install_simple();
    tracer.in_span("doing_work", |cx| {
        // Traced app logic here...
    });
    shutdown_tracer_provider(); // sending remaining spans
}
