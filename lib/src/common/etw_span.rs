pub trait EtwSpan {
    fn get_span_data(&self) -> &opentelemetry_sdk::export::trace::SpanData;
}
