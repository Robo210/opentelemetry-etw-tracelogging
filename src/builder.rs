use crate::batch_exporter::*;
use crate::realtime_exporter::*;
use opentelemetry_api::{global, trace::TracerProvider};
use opentelemetry_sdk::trace::Builder;
use tracelogging_dynamic::Guid;

#[derive(Debug)]
pub struct EtwExporterBuilder {
    provider_name: String,
    provider_id: Guid,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_etw_exporter(name: &str) -> EtwExporterBuilder {
    EtwExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        trace_config: None,
    }
}

impl EtwExporterBuilder {
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id(mut self, guid: &Guid) -> Self {
        self.provider_id = guid.to_owned();
        self
    }

    /// Assign the SDK trace configuration.
    pub fn with_trace_config(mut self, config: opentelemetry_sdk::trace::Config) -> Self {
        self.trace_config = Some(config);
        self
    }

    /// Install the exporter as a "simple" span exporter.
    /// Spans will be automatically batched and exported some time after
    /// the span has ended. The timestamps of the ETW events, and the
    /// duration of time between events, will not be accurate.
    pub fn install_simple(self) -> opentelemetry_sdk::trace::Tracer {
        let exporter = BatchExporter::new(&self.provider_name);

        let provider_builder =
            opentelemetry_sdk::trace::TracerProvider::builder().with_simple_exporter(exporter);

        self.install(provider_builder)
    }

    /// Install the exporter as a span processor.
    /// Spans will be exported almost immediately after they are started
    /// and ended. Events that were added to the span will be exported
    /// at the same time as the end event. The timestamps of the start
    /// and end ETW events will roughly match the actual start and end of the span.
    pub fn install_realtime(self) -> opentelemetry_sdk::trace::Tracer {
        let exporter = RealtimeExporter::new(&self.provider_name);

        let provider_builder =
            opentelemetry_sdk::trace::TracerProvider::builder().with_span_processor(exporter);

        self.install(provider_builder)
    }

    fn install(mut self, provider_builder: Builder) -> opentelemetry_sdk::trace::Tracer {
        let builder = if let Some(config) = self.trace_config.take() {
            provider_builder.with_config(config)
        } else {
            provider_builder
        };

        let provider = builder.build();

        let tracer =
            provider.versioned_tracer("opentelemetry-etw", Some(env!("CARGO_PKG_VERSION")), None);
        let _ = global::set_tracer_provider(provider);

        tracer
    }
}
