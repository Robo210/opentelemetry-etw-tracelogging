use crate::batch_exporter::*;
use crate::realtime_exporter::*;
use opentelemetry_api::{global, trace::TracerProvider};
use opentelemetry_sdk::trace::Builder;
use tracelogging_dynamic::Guid;

#[derive(Debug)]
pub struct EtwExporterBuilder {
    provider_name: String,
    provider_id: Guid,
    use_byte_for_bools: bool,
    #[cfg(feature = "json")]
    json: bool,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_etw_exporter(name: &str) -> EtwExporterBuilder {
    EtwExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        use_byte_for_bools: false,
        #[cfg(feature = "json")]
        json: false,
        trace_config: None,
    }
}

impl EtwExporterBuilder {
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id(mut self, guid: Guid) -> Self {
        self.provider_id = guid;
        self
    }

    /// Get the current provider ID that will be used for the ETW provider.
    /// This is a convenience function to help with tools that do not implement
    /// the standard provider name to ID  algorithm.
    pub fn get_provider_id(&self) -> Guid {
        self.provider_id
    }

    /// Log bool attributes using an InType of `xs:byte` instead of `win:Boolean`.
    /// This is non-standard and not recommended except if compatability with the
    /// C++ ETW exporter is required.
    pub fn with_byte_sized_bools(mut self) -> Self {
        self.use_byte_for_bools = true;
        self
    }

    /// Assign the SDK trace configuration.
    pub fn with_trace_config(mut self, config: opentelemetry_sdk::trace::Config) -> Self {
        self.trace_config = Some(config);
        self
    }

    /// Encode the event payload as a single JSON string rather than multiple fields.
    /// Recommended only for compatability with the C++ ETW exporter.
    /// Requires the `json` feature to be enabled on the crate.
    #[cfg(any(feature = "json", doc))]
    pub fn with_json_payload(mut self) -> Self {
        self.json = true;
        self
    }

    /// Install the exporter as a "simple" span exporter.
    /// Spans will be automatically batched and exported some time after
    /// the span has ended. The timestamps of the ETW events, and the
    /// duration of time between events, will not be accurate.
    pub fn install_simple(self) -> opentelemetry_sdk::trace::Tracer {
        let exporter = BatchExporter::new(&self.provider_name, self.use_byte_for_bools, self.json);

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
        let exporter = RealtimeExporter::new(&self.provider_name, self.use_byte_for_bools, self.json);

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

#[allow(unused_imports)]
mod tests {
    use super::*;

    #[test]
    fn create_builder() {
        let builder = new_etw_exporter("my_provider_name");
        assert!(
            builder.get_provider_id()
                == Guid::from_fields(
                    0x6386e494,
                    0x0d79,
                    0x57b7,
                    [0xa8, 0x38, 0x16, 0x8d, 0xc4, 0x19, 0xf5, 0x42]
                )
        );

        let builder = builder.with_provider_id(Guid::from_fields(
            0x1fa0f771,
            0x9607,
            0x4fe2,
            [0x8c, 0x39, 0x2b, 0x6c, 0x61, 0x43, 0xbb, 0x87],
        ));
        assert!(
            builder.get_provider_id()
                == Guid::from_fields(
                    0x1fa0f771,
                    0x9607,
                    0x4fe2,
                    [0x8c, 0x39, 0x2b, 0x6c, 0x61, 0x43, 0xbb, 0x87]
                )
        );
    }

    #[test]
    fn install_simple() {
        new_etw_exporter("my_provider_name").install_simple();
    }

    #[test]
    fn install_realtime() {
        new_etw_exporter("my_provider_name").install_realtime();
    }
}
